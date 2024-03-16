use crossbeam_channel as channel;
use indexmap::IndexMap as Map;
use isahc::{
    config::{Configurable, RedirectPolicy},
    HttpClient,
};
use once_cell::sync::Lazy;
use onig::Regex;
use rayon::prelude::*;
use select::{
    document::Document,
    predicate::{Class, Name},
};
use std::{
    borrow::Cow,
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

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.3";
static USER_AGENT: Lazy<Cow<'static, str>> = Lazy::new(|| {
    env::var("USER_AGENT")
        .map(Cow::from)
        .unwrap_or_else(|_| Cow::from(DEFAULT_USER_AGENT))
});

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
                ("de", "deDE", format!("L = BigWigs:NewBossLocale(\"{module_name}\", \"deDE\")")),
                ("es", "esES", format!("L = BigWigs:NewBossLocale(\"{module_name}\", \"esES\") or BigWigs:NewBossLocale(\"{module_name}\", \"esMX\")")),
                ("fr", "frFR", format!("L = BigWigs:NewBossLocale(\"{module_name}\", \"frFR\")")),
                ("it", "itIT", format!("L = BigWigs:NewBossLocale(\"{module_name}\", \"itIT\")")),
                ("pt", "ptBR", format!("L = BigWigs:NewBossLocale(\"{module_name}\", \"ptBR\")")),
                ("ru", "ruRU", format!("L = BigWigs:NewBossLocale(\"{module_name}\", \"ruRU\")")),
                ("ko", "koKR", format!("L = BigWigs:NewBossLocale(\"{module_name}\", \"koKR\")")),
                ("cn", "zhCN", format!("L = BigWigs:NewBossLocale(\"{module_name}\", \"zhCN\")")),
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
    fn get_tmp_dir(output_dir: &Path) -> Cow<'_, Path> {
        use std::fs;
        use std::os::unix::fs::MetadataExt;

        let os_tmp = env::temp_dir();
        match (
            fs::metadata(output_dir).map(|v| v.dev()),
            fs::metadata(&os_tmp).map(|v| v.dev()),
        ) {
            (Ok(num1), Ok(num2)) if num1 == num2 => Cow::from(os_tmp),
            _ => Cow::from(output_dir),
        }
    }

    #[cfg(windows)]
    fn get_tmp_dir(output_dir: &Path) -> Cow<'_, Path> {
        use winapi_util::{file, Handle};

        let os_tmp = env::temp_dir();
        let serial_num = |path: &Path| {
            Handle::from_path_any(path)
                .and_then(|h| file::information(h))
                .map(|v| v.volume_serial_number())
        };

        match (serial_num(output_dir), serial_num(&os_tmp)) {
            (Ok(num1), Ok(num2)) if num1 == num2 => Cow::from(os_tmp),
            _ => Cow::from(output_dir),
        }
    }

    #[cfg(not(any(unix, windows)))]
    #[inline(always)]
    fn get_tmp_dir(output_dir: &Path) -> Cow<'_, Path> {
        Cow::from(output_dir)
    }

    fn process_languages(self) {
        static TITLE_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r#"\s+<.+?>$"#).unwrap());

        let total = self.data.iter().fold(0, |acc, el| acc + el.ids_map.len());

        if total > 0 {
            let (tx, rx) = channel::bounded(total);

            let stderr_thread = thread::spawn(move || {
                let stderr = std::io::stderr();
                let mut stderr = stderr.lock();
                let mut processed = 0;

                let _ = write!(stderr, "\rProgress: 0 / {total}");
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
                                "\rFailed to collect data for \"{mob_name}\" ({language}), error: {e}"
                            );
                            processed += 1;
                        }
                        _ => processed += 1,
                    }
                    let _ = write!(stderr, "\rProgress: {processed} / {total}");
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
                        .default_header("sec-fetch-site", "same-site")
                        .default_header("sec-fetch-user", "?1")
                        .default_header("upgrade-insecure-requests", "1")
                        .default_header("user-agent", &**USER_AGENT)
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
                                    .get(&format!("https://{subdomain}.wowhead.com/npc={id}"))
                                    .map_err(From::from)
                                    .and_then(|mut response| {
                                        Document::from_read(response.body_mut()).map_err(From::from)
                                    })
                                    .and_then(|document| {
                                        document
                                            .find(Class("heading-size-1"))
                                            .next()
                                            .ok_or_else(|| {
                                                "Couldn't find an element .heading-size-1".into()
                                            })
                                            .and_then(|node| {
                                                // Check if we were redirected to the search page.

                                                if let Some(parent) = node.parent().and_then(|n| n.parent()) {
                                                    if parent.is(Name("form")) {
                                                        return Err("Not a valid NPC ID".into());
                                                    }

                                                    for child in parent.children() {
                                                        if child.is(Class("database-detail-page-not-found-message")) {
                                                            return Err("Not a valid NPC ID".into());
                                                        }
                                                    }
                                                }

                                                Ok(node.text())
                                            })
                                    });

                                match result {
                                    Ok(translation) => {
                                        let _ = tx.send(Ok(()));
                                        let translation =
                                            utils::replace_owning(translation, &TITLE_REGEX, "");
                                        let translation = if translation.contains('\"') {
                                            translation.replace('\"', "\\\"")
                                        } else {
                                            translation
                                        };
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

            if let Err(e) = File::open(&output_dir).and_then(|dir| dir.sync_all()) {
                eprintln!(
                    "Failed to call fsync() on \"{}\": {}",
                    output_dir.display(),
                    e
                );
            }
        } else {
            eprintln!("There's nothing to do.");
        }
    }
}
