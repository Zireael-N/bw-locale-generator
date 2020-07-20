use crossbeam_channel as channel;
use indexmap::IndexMap as Map;
use isahc::config::RedirectPolicy;
use isahc::prelude::*;
use rayon::prelude::*;
use select::{document::Document, predicate::Class};
use std::{
    env,
    fs::File,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

mod error;
pub use error::Error;
use error::ProcessingError;
mod utils;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/83.0.4103.116 Safari/537.36";

#[derive(Debug, Clone)]
pub struct LanguageData {
    subdomain: &'static str,
    code: &'static str,
    header: String,
    ids_map: Map<String, i64>,
}

#[derive(Debug, Clone)]
pub struct Localizer {
    data: Vec<LanguageData>,
    output_dir: PathBuf,
}

impl Localizer {
    pub fn run<P: Into<PathBuf>>(
        ids_map: Map<String, i64>,
        module_name: &str,
        output_dir: P,
        force_all: bool,
    ) {
        let output_dir = output_dir.into();
        let localizer = Self {
            data: Self::construct_language_data(vec![
                // ("www", "enUS", String::from("L = mod:GetLocale()")),
                ("de", "deDE", format!("L = BigWigs:NewBossLocale(\"{}\", \"deDE\")", module_name)),
                ("es", "esES", format!("L = BigWigs:NewBossLocale(\"{name}\", \"esES\") or BigWigs:NewBossLocale(\"{name}\", \"esMX\")", name=module_name)),
                ("fr", "frFR", format!("L = BigWigs:NewBossLocale(\"{}\", \"frFR\")", module_name)),
                ("it", "itIT", format!("L = BigWigs:NewBossLocale(\"{}\", \"itIT\")", module_name)),
                ("pt", "ptBR", format!("L = BigWigs:NewBossLocale(\"{}\", \"ptBR\")", module_name)),
                ("ru", "ruRU", format!("L = BigWigs:NewBossLocale(\"{}\", \"ruRU\")", module_name)),
                ("ko", "koKR", format!("L = BigWigs:NewBossLocale(\"{}\", \"koKR\")", module_name)),
                ("cn", "zhCN", format!("L = BigWigs:NewBossLocale(\"{}\", \"zhCN\")", module_name)),
            ], &ids_map, if force_all { None } else { Some(&output_dir) }),
            output_dir,
        };

        localizer.process_languages();
    }

    fn construct_language_data(
        initial_data: Vec<(&'static str, &'static str, String)>,
        ids_map: &Map<String, i64>,
        output_dir: Option<&Path>,
    ) -> Vec<LanguageData> {
        initial_data
            .into_par_iter()
            .filter_map(|language| {
                let mut ids_map = ids_map.clone();

                if let Some(output_dir) = output_dir {
                    let file_path = output_dir.join(format!("{}.lua", language.1));
                    if let Ok(file) = File::open(file_path) {
                        let mut file = BufReader::new(file);
                        let _ = utils::discard_existing(&mut file, &language.2, &mut ids_map);
                    }
                }

                if ids_map.is_empty() {
                    None
                } else {
                    Some(LanguageData {
                        subdomain: language.0,
                        code: language.1,
                        header: language.2,
                        ids_map,
                    })
                }
            })
            .collect()
    }

    #[cfg(unix)]
    fn get_tmp_dir(output_dir: &Path) -> PathBuf {
        use std::fs;
        use std::os::unix::fs::MetadataExt;

        let os_tmp = env::temp_dir();
        match (
            fs::metadata(output_dir).map(|v| v.dev()),
            fs::metadata(&os_tmp).map(|v| v.dev()),
        ) {
            (Ok(num1), Ok(num2)) if num1 == num2 => os_tmp,
            _ => output_dir.to_path_buf(),
        }
    }

    #[cfg(windows)]
    fn get_tmp_dir(output_dir: &Path) -> PathBuf {
        use winapi_util::{file, Handle};

        let os_tmp = env::temp_dir();
        let serial_num = |path: &Path| {
            Handle::from_path_any(path)
                .and_then(|h| file::information(h))
                .map(|v| v.volume_serial_number())
        };

        match (serial_num(output_dir), serial_num(&os_tmp)) {
            (Ok(num1), Ok(num2)) if num1 == num2 => os_tmp,
            _ => output_dir.to_path_buf(),
        }
    }

    #[cfg(not(any(unix, windows)))]
    #[inline(always)]
    fn get_tmp_dir(output_dir: &Path) -> PathBuf {
        output_dir.to_path_buf()
    }

    fn process_languages(self) {
        let total = self.data.iter().fold(0, |acc, el| acc + el.ids_map.len());

        if total > 0 {
            let (tx, rx) = channel::bounded(total);

            let stderr_thread = thread::spawn(move || {
                let stderr = std::io::stderr();
                let mut stderr = stderr.lock();
                let mut processed = 0;

                let _ = write!(stderr, "\rProgress: 0 / {}", total);
                while let Ok(msg) = rx.recv() {
                    match msg {
                        Err(ProcessingError::IoError((path, e))) => {
                            let _ = writeln!(
                                stderr,
                                "\rI/O error: {} ({})",
                                e,
                                path.to_string_lossy(),
                            );
                        }
                        Err(ProcessingError::DataError((language, mob_name, e))) => {
                            let _ = writeln!(
                                stderr,
                                "\rFailed to collect data for \"{}\" ({}), error: {}",
                                mob_name, language, e
                            );
                            processed += 1;
                        }
                        _ => processed += 1,
                    }
                    let _ = write!(stderr, "\rProgress: {} / {}", processed, total);
                }

                let _ = stderr.write(b"\n");
                let _ = stderr.flush();
            });

            let output_dir = self.output_dir;
            let tmp_dir = Self::get_tmp_dir(&output_dir);
            self.data.into_par_iter().for_each({
                |language| {
                    let client = HttpClient::builder()
                        .timeout(Duration::from_secs(30))
                        .redirect_policy(RedirectPolicy::Limit(5))
                        .default_header(
                            "accept",
                            "text/html,application/xhtml+xml,application/xml;q=0.9",
                        )
                        .default_header("accept-encoding", "gzip, deflate")
                        .default_header("accept-language", "en-US,en;q=0.9")
                        .default_header("sec-fetch-dest", "document")
                        .default_header("sec-fetch-mode", "navigate")
                        .default_header("sec-fetch-site", "none")
                        .default_header("sec-fetch-user", "?1")
                        .default_header("upgrade-insecure-requests", "1")
                        .default_header("user-agent", USER_AGENT)
                        .build()
                        .unwrap();

                    let code = language.code;
                    let subdomain = language.subdomain;
                    let map: Map<_, _> = language
                        .ids_map
                        .into_iter()
                        .filter_map({
                            let client = &client;
                            let tx = tx.clone();

                            move |(name, id)| {
                                let result: Result<_, Error> = client
                                    .get(&format!("https://{}.wowhead.com/npc={}", subdomain, id,))
                                    .map_err(From::from)
                                    .and_then(|mut response| {
                                        Document::from_read(response.body_mut()).map_err(From::from)
                                    })
                                    .and_then(|document| {
                                        document
                                            .find(Class("heading-size-1"))
                                            .next()
                                            .map(|node| node.text())
                                            .ok_or_else(|| {
                                                "Couldn't find an element .heading-size-1".into()
                                            })
                                    });

                                match result {
                                    Ok(translation) => {
                                        let _ = tx.send(Ok(()));
                                        let (translation, is_valid) = match translation.as_bytes() {
                                            [b'[', rest @ .., b']'] => {
                                                (String::from_utf8(rest.to_vec()).unwrap(), false)
                                            }
                                            _ => (translation, true),
                                        };
                                        Some((name, (translation, is_valid)))
                                    }
                                    Err(e) => {
                                        let _ = tx
                                            .send(Err(ProcessingError::DataError((code, name, e))));
                                        None
                                    }
                                }
                            }
                        })
                        .collect();

                    if let Err(e) = utils::write_to_dir(
                        &output_dir,
                        &tmp_dir,
                        language.code,
                        &language.header,
                        map,
                    ) {
                        let _ = tx.send(Err(ProcessingError::IoError(e)));
                    }
                }
            });

            drop(tx);
            stderr_thread.join().unwrap();
        } else {
            eprintln!("There's nothing to do.");
        }
    }
}
