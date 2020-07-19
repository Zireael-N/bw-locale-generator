use once_cell::sync::Lazy;
use regex::Regex;

use crate::Map;
use std::{
    borrow::Cow,
    env,
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[cfg(windows)]
const LINE_ENDING: &'static [u8] = b"\r\n";
#[cfg(not(windows))]
const LINE_ENDING: &'static [u8] = b"\n";

enum State {
    Initial,
    FoundLocale,
    InsideIf,
    Done,
}

static LOCALE_ASSIGNMENT_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\s*(--)?\s*L\.((?-u)\w*)\s*=\s*"(.*?)"(.*)"#).unwrap());

fn offset<'a>(haystack: &'a str, needle: &'a str) -> usize {
    needle.as_ptr() as usize - haystack.as_ptr() as usize
}

pub(crate) fn discard_existing(
    file: &mut BufReader<File>,
    header: &str,
    map: &mut Map<String, i64>,
) -> Result<(), io::Error> {
    let mut state = State::Initial;

    for line in file.lines() {
        let line = line?;

        match state {
            State::Initial => {
                if line.trim().contains(header) {
                    state = State::FoundLocale;
                }
            }
            State::FoundLocale => {
                let line = line.trim();
                if line == "if L then" {
                    state = State::InsideIf;
                } else if line.starts_with("L =") && line != header {
                    state = State::Initial;
                }
            }
            State::InsideIf => {
                if line.trim() == "end" {
                    break;
                } else if let Some(caps) = LOCALE_ASSIGNMENT_REGEX.captures(&line) {
                    let is_comment = caps.get(1).is_some();
                    let name = caps.get(2).unwrap().as_str();

                    if !is_comment {
                        let _ = map.remove(name);
                    }
                }
            }
            _ => (),
        }
    }

    Ok(())
}

fn replace<'a, 'b>(
    src: &'a str,
    header: &'b str,
    mut values: Map<String, (String, bool)>,
) -> Option<Cow<'a, str>> {
    let mut state = State::Initial;
    let mut scratch: Vec<u8> = Vec::new();
    let mut copy_from = 0;

    let bytes = src.as_bytes();
    for line in src.lines() {
        match state {
            State::Initial => {
                if line.trim().contains(header) {
                    state = State::FoundLocale;
                }
            }
            State::FoundLocale => {
                let line = line.trim();
                if line.trim() == "if L then" {
                    state = State::InsideIf;
                } else if line.starts_with("L =") && line != header {
                    state = State::Initial;
                }
            }
            State::InsideIf => {
                if line.trim() == "end" {
                    if !values.is_empty() {
                        let offset = offset(src, line);

                        scratch.extend_from_slice(&bytes[copy_from..offset]);
                        for (name, (translation, is_valid)) in &values {
                            scratch.extend_from_slice(
                                format!(
                                    "{}L.{} = \"{}\"",
                                    if *is_valid { "\t" } else { "\t -- " },
                                    name,
                                    translation
                                )
                                .as_bytes(),
                            );
                            scratch.extend_from_slice(LINE_ENDING);
                        }
                        copy_from = offset;
                    }
                    state = State::Done;
                    break;
                } else if let Some(caps) = LOCALE_ASSIGNMENT_REGEX.captures(line) {
                    let name = caps.get(2).unwrap().as_str();

                    if let Some((translation, is_valid)) = values.remove(name) {
                        let is_comment = caps.get(1).is_some();
                        let leftover = caps.get(4).unwrap().as_str();
                        if is_valid && (is_comment || caps.get(3).unwrap().as_str() != translation)
                        {
                            let offset = offset(src, line);

                            scratch.extend_from_slice(&bytes[copy_from..offset]);
                            scratch.extend_from_slice(
                                format!("\tL.{} = \"{}\"{}", name, translation, leftover)
                                    .as_bytes(),
                            );
                            copy_from = offset + line.len();
                        }
                    }
                }
            }
            _ => (),
        }
    }

    match state {
        State::Done => {
            if scratch.is_empty() {
                Some(Cow::from(src))
            } else {
                scratch.extend_from_slice(&bytes[copy_from..]);
                String::from_utf8(scratch).map(Cow::from).ok()
            }
        }
        _ => {
            let is_empty = src.trim().is_empty();

            if is_empty {
                scratch.extend_from_slice(b"local ");
            } else {
                scratch.extend_from_slice(&bytes[copy_from..]);
                scratch.extend_from_slice(LINE_ENDING);
            }

            scratch.extend_from_slice(header.as_bytes());
            scratch.extend_from_slice(LINE_ENDING);

            if is_empty {
                scratch.extend_from_slice(b"if not L then return end");
                scratch.extend_from_slice(LINE_ENDING);
            }

            scratch.extend_from_slice(b"if L then");
            scratch.extend_from_slice(LINE_ENDING);

            for (name, (translation, is_valid)) in values {
                scratch.extend_from_slice(
                    format!(
                        "{}L.{} = \"{}\"",
                        if is_valid { "\t" } else { "\t -- " },
                        name,
                        translation
                    )
                    .as_bytes(),
                );
                scratch.extend_from_slice(LINE_ENDING);
            }
            scratch.extend_from_slice(b"end");
            scratch.extend_from_slice(LINE_ENDING);
            String::from_utf8(scratch).map(Cow::from).ok()
        }
    }
}

pub(crate) fn write_to_dir(
    output_dir: &Path,
    language_code: &str,
    header: &str,
    values: Map<String, (String, bool)>,
) -> Result<(), (PathBuf, io::Error)> {
    let to_path = output_dir.join(format!("{}.lua", language_code));
    match File::open(&to_path) {
        // File exists, replace its contents if needed.
        Ok(mut to_file) => {
            let contents = {
                let mut s = String::new();
                to_file
                    .read_to_string(&mut s)
                    .map_err(|e| (to_path.clone(), e))?;
                s
            };

            // If we didn't change anything, quit early.
            if let Cow::Owned(replaced) = replace(&contents, header, values).unwrap() {
                let unix_ts = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                // Renaming a file is an atomic operation, writing to it is not.
                // Create a temporary file and then rename it to prevent leaving an existing file in a bad state.
                let tmp_path = env::temp_dir().join(format!("{}-{}.lua", language_code, unix_ts));
                let mut tmp_file = File::create(&tmp_path).map_err(|e| (tmp_path.clone(), e))?;
                tmp_file
                    .write_all(replaced.as_bytes())
                    .map_err(|e| (tmp_path.clone(), e))?;
                tmp_file.flush().map_err(|e| (tmp_path.clone(), e))?;

                drop(to_file);

                // Fails if files belong to different filesystems
                if let Err(_) = fs::rename(&tmp_path, &to_path) {
                    let copy_result = fs::copy(&tmp_path, &to_path);
                    fs::remove_file(&tmp_path).map_err(|e| (tmp_path, e))?;

                    if let Err(e) = copy_result {
                        return Err((to_path, e));
                    }
                }
            }
        }
        Err(e) => {
            if e.kind() == ErrorKind::NotFound {
                let mut to_file =
                    BufWriter::new(File::create(&to_path).map_err(|e| (to_path.clone(), e))?);

                to_file
                    .write_all(b"local ")
                    .map_err(|e| (to_path.clone(), e))?;
                to_file
                    .write_all(header.as_bytes())
                    .map_err(|e| (to_path.clone(), e))?;
                to_file
                    .write_all(LINE_ENDING)
                    .map_err(|e| (to_path.clone(), e))?;
                to_file
                    .write_all(b"if not L then return end")
                    .map_err(|e| (to_path.clone(), e))?;
                to_file
                    .write_all(LINE_ENDING)
                    .map_err(|e| (to_path.clone(), e))?;
                to_file
                    .write_all(b"if L then")
                    .map_err(|e| (to_path.clone(), e))?;
                to_file
                    .write_all(LINE_ENDING)
                    .map_err(|e| (to_path.clone(), e))?;

                for (name, (translation, is_valid)) in values {
                    to_file
                        .write_all(
                            format!(
                                "{}L.{} = \"{}\"",
                                if is_valid { "\t" } else { "\t -- " },
                                name,
                                translation
                            )
                            .as_bytes(),
                        )
                        .map_err(|e| (to_path.clone(), e))?;
                    to_file
                        .write_all(LINE_ENDING)
                        .map_err(|e| (to_path.clone(), e))?;
                }

                to_file
                    .write_all(b"end")
                    .map_err(|e| (to_path.clone(), e))?;
                to_file
                    .write_all(LINE_ENDING)
                    .map_err(|e| (to_path.clone(), e))?;
                to_file.flush().map_err(|e| (to_path, e))?;
            } else {
                // Insufficient permissions or whatever else.
                return Err((to_path, e));
            }
        }
    }

    Ok(())
}
