use crate::Map;
use onig::{Regex, Replacer};
use std::{
    borrow::Cow,
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::SystemTime,
};

#[cfg(windows)]
const LINE_ENDING: &[u8] = b"\r\n";
#[cfg(not(windows))]
const LINE_ENDING: &[u8] = b"\n";

enum State {
    Initial,
    FoundLocale,
    InsideIf,
    Done,
}

static LOCALE_ASSIGNMENT_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\s*(--)?\s*L\.(\w*)\s*=\s*"(.*?)(?<!\\)"(.*)"#).unwrap());

fn offset<'a>(haystack: &'a str, needle: &'a str) -> usize {
    needle.as_ptr().addr() - haystack.as_ptr().addr()
}

pub(crate) fn replace_owning<R: Replacer>(
    source: String,
    regex: &Regex,
    mut replacement: R,
) -> String {
    if let Some(cap) = regex.captures_iter(&source).next() {
        let mut new = String::with_capacity(source.len());

        let (start, end) = cap.pos(0).unwrap();
        new.push_str(&source[..start]);
        new.push_str(&replacement.reg_replace(&cap));
        new.push_str(&source[end..]);

        new
    } else {
        source
    }
}

pub(crate) fn discard_existing(
    file: &mut BufReader<File>,
    header: &str,
    map: &mut Map<String, i64>,
) -> Result<(), io::Error> {
    let mut state = State::Initial;

    let mut line = String::new();
    while file.read_line(&mut line)? > 0 {
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
                    let is_comment = caps.at(1).is_some();
                    let name = caps.at(2).unwrap();

                    if !is_comment {
                        let _ = map.shift_remove(name);
                    }
                }
            }
            _ => (),
        }
        line.clear();
    }

    Ok(())
}

fn replace<'a>(
    src: &'a str,
    header: &str,
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
                                    if *is_valid { "\t" } else { "\t-- " },
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
                    let name = caps.at(2).unwrap();

                    if let Some((translation, is_valid)) = values.shift_remove(name) {
                        let is_comment = caps.at(1).is_some();
                        let leftover = caps.at(4).unwrap();
                        if is_valid && (is_comment || caps.at(3).unwrap() != translation) {
                            let offset = offset(src, line);

                            scratch.extend_from_slice(&bytes[copy_from..offset]);
                            scratch.extend_from_slice(
                                format!("\tL.{name} = \"{translation}\"{leftover}").as_bytes(),
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
                        if is_valid { "\t" } else { "\t-- " },
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
    tmp_dir: &Path,
    language_code: &str,
    header: &str,
    values: Map<String, (String, bool)>,
) -> Result<(), (PathBuf, io::Error)> {
    let to_path = output_dir.join(format!("{language_code}.lua"));
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
                let tmp_path = tmp_dir.join(format!("{language_code}-{unix_ts}.lua.tmp"));
                let mut tmp_file = File::create(&tmp_path).map_err(|e| (tmp_path.clone(), e))?;
                tmp_file
                    .write_all(replaced.as_bytes())
                    .map_err(|e| (tmp_path.clone(), e))?;
                tmp_file.sync_all().map_err(|e| (tmp_path.clone(), e))?;

                drop(to_file);
                drop(tmp_file);

                // Fails if files belong to different filesystems
                if fs::rename(&tmp_path, &to_path).is_err() {
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
                    write!(
                        to_file,
                        "{}L.{} = \"{}\"",
                        if is_valid { "\t" } else { "\t-- " },
                        name,
                        translation
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

                to_file.flush().map_err(|e| (to_path.clone(), e))?;
                if let Ok(to_file) = to_file.into_inner() {
                    to_file.sync_all().map_err(|e| (to_path, e))?;
                }
            } else {
                // Insufficient permissions or whatever else.
                return Err((to_path, e));
            }
        }
    }

    Ok(())
}
