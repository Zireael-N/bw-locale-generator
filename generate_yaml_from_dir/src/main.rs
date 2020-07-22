use indexmap::IndexMap as Map;
use once_cell::sync::Lazy;
use rayon::prelude::*;
use regex::Regex;
use std::{
    env, fmt,
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::PathBuf,
};
use walkdir::WalkDir;

#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

struct ParseResult {
    module_name: Option<String>,
    var_to_id_map: Map<String, i64>,
    missing_vars: Vec<(String, String)>,
    missing_ids: Vec<(i64, String)>,
}

impl fmt::Debug for ParseResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParseResult")
            .field("Variable-to-ID map", &self.var_to_id_map)
            .field("Missing variables", &self.missing_vars)
            .field("Missing IDs", &self.missing_ids)
            .finish()
    }
}

#[derive(Copy, Clone, Eq, Debug, PartialEq, PartialOrd, Ord)]
enum ParseState {
    ParsingIds,
    ParsingVars,
    Neither,
}

fn parse(input: BufReader<File>) -> ParseResult {
    static IDS_START: &str = "mod:RegisterEnableMob(";
    static VARS_START: &str = "if L then";

    static ID_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^\s*(\d+),?\s*--\s*(.+)$"#).unwrap());
    static VAR_REGEX: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"^\s*L\.(\w+)\s*=\s*"(.+?)""#).unwrap());
    static MODULE_DECL_REGEX: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^\s*local\s*mod(?:,\s*CL)?\s*=\s*BigWigs:NewBoss\("(.*?)(?: Trash)""#)
            .unwrap()
    });

    let mut ids_map = Map::with_capacity(16);
    let mut vars_map = Map::with_capacity(16);
    let mut module_name = None;

    let mut state = ParseState::Neither;
    let mut parsed_blocks = 0;

    for line in input.lines().filter_map(Result::ok) {
        match state {
            ParseState::ParsingIds => match ID_REGEX.captures(&line) {
                Some(caps) => {
                    ids_map.insert(
                        caps.get(2).unwrap().as_str().trim().to_string(),
                        caps.get(1)
                            .map(|cap| cap.as_str())
                            .and_then(|cap| cap.parse::<i64>().ok())
                            .unwrap(),
                    );
                }
                None => {
                    if line.find(')').is_some() {
                        state = ParseState::Neither;
                        if parsed_blocks == 2 {
                            break;
                        }
                        parsed_blocks += 1;
                    }
                }
            },
            ParseState::ParsingVars => match VAR_REGEX.captures(&line) {
                Some(caps) => {
                    vars_map.insert(
                        caps.get(2).unwrap().as_str().to_string(),
                        caps.get(1).unwrap().as_str().to_string(),
                    );
                }
                None => {
                    if line == "end" {
                        state = ParseState::Neither;
                        if parsed_blocks == 2 {
                            break;
                        }
                        parsed_blocks += 1;
                    }
                }
            },
            ParseState::Neither => {
                if line.starts_with(IDS_START) {
                    state = ParseState::ParsingIds;
                } else if line.starts_with(VARS_START) {
                    state = ParseState::ParsingVars;
                } else if let Some(caps) = MODULE_DECL_REGEX.captures(&line) {
                    module_name = caps.get(1).map(|v| v.as_str().into());
                    parsed_blocks += 1;
                }
            }
        }
    }

    let mut var_to_id_map = Map::with_capacity(vars_map.len());
    let mut missing_vars = Vec::with_capacity(4);

    for (value, variable) in vars_map.into_iter() {
        if let Some(id) = ids_map.remove(&value) {
            var_to_id_map.insert(variable, id);
        } else {
            missing_vars.push((variable, value));
        }
    }

    let missing_ids: Vec<_> = ids_map
        .into_iter()
        .map(|(comment, id)| (id, comment))
        .collect();

    ParseResult {
        module_name,
        var_to_id_map,
        missing_vars,
        missing_ids,
    }
}

fn write_to_file(parse_result: &ParseResult, mut output: BufWriter<File>) -> Result<(), io::Error> {
    for (variable, id) in parse_result.var_to_id_map.iter() {
        writeln!(output, "{}: {}", variable, id)?;
    }

    output.flush()
}

fn print_errors(
    results: Vec<Result<(PathBuf, ParseResult), (PathBuf, Error)>>,
) -> Result<(), std::io::Error> {
    let mut dirty = false;

    let stderr = std::io::stderr();
    let mut stderr = stderr.lock();

    for result in results.into_iter() {
        match result {
            Ok((path, parse_result)) => {
                let ids_missing = !parse_result.missing_ids.is_empty();
                let vars_missing = !parse_result.missing_vars.is_empty();

                if ids_missing || vars_missing {
                    if dirty {
                        stderr.write_all(b"\n==========\n\n")?;
                    }
                    write!(stderr, "{}", path.display())?;

                    if vars_missing {
                        stderr.write_all(b"\nMissing variables:\n")?;
                        for (variable, value) in parse_result.missing_vars.iter() {
                            writeln!(stderr, "{} (\"{}\")", variable, value)?;
                        }
                    }

                    if ids_missing {
                        stderr.write_all(b"\nMissing IDs:\n")?;
                        for (id, comment) in parse_result.missing_ids.iter() {
                            writeln!(stderr, "{} (\"{}\")", id, comment)?;
                        }
                    }

                    dirty = true;
                }
            }
            Err((path, err)) => {
                if dirty {
                    stderr.write_all(b"\n==========\n\n")?;
                }
                writeln!(
                    stderr,
                    "Error while working on {}: {:?}",
                    path.display(),
                    err
                )?;

                dirty = true;
            }
        }
    }

    stderr.flush()
}

fn main() -> Result<(), Error> {
    let (input_dir, output_dir) = {
        let mut args = env::args_os();
        let program_name = args.next().unwrap();

        match (args.next(), args.next()) {
            (Some(input_dir), Some(output_dir)) => (input_dir, PathBuf::from(output_dir)),
            _ => {
                eprintln!(
                    "Usage: {} input_directory output_directory",
                    program_name.to_string_lossy()
                );
                std::process::exit(1);
            }
        }
    };

    let file_paths = WalkDir::new(&input_dir)
        .into_iter()
        .filter_map(|entry| match entry {
            Ok(entry) => {
                let path = entry.into_path();
                if path.ends_with("Trash.lua") {
                    Some(Ok(path))
                } else {
                    None
                }
            }
            Err(err) => Some(Err(err)),
        })
        .collect::<Result<Vec<_>, _>>()?;

    let results: Vec<_> = file_paths
        .into_par_iter()
        .map(|input_path| -> Result<_, (_, Error)> {
            let output_path = {
                input_path
                    .strip_prefix(&input_dir)
                    .map_err(|e| (input_path.clone(), From::from(e)))
                    .and_then(|path| {
                        path.parent().ok_or_else(|| {
                            (
                                input_path.clone(),
                                "Failed to get a parent directory".into(),
                            )
                        })
                    })
                    .map(|path| output_dir.join(path).with_extension("yaml"))
            }?;

            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(&parent).map_err(|e| (input_path.clone(), From::from(e)))?;
            }

            let input = BufReader::new(
                File::open(&input_path).map_err(|e| (input_path.clone(), From::from(e)))?,
            );

            let parse_result = parse(input);

            if parse_result.var_to_id_map.is_empty() {
                Ok((input_path, parse_result))
            } else {
                let output_file = match parse_result.module_name {
                    Some(ref v) => {
                        match File::create(output_path.with_file_name(format!("{}.yaml", v))) {
                            Ok(file) => Ok(file),
                            Err(_) => File::create(&output_path)
                                .map_err(|e| (input_path.clone(), From::from(e))),
                        }
                    }
                    None => {
                        File::create(&output_path).map_err(|e| (input_path.clone(), From::from(e)))
                    }
                };

                let output_file = BufWriter::new(output_file?);

                write_to_file(&parse_result, output_file)
                    .map_err(|e| (input_path.clone(), From::from(e)))
                    .map(|_| (input_path, parse_result))
            }
        })
        .collect();

    match env::var_os("SHOW_MISSING_IDS_AND_VARS") {
        Some(ref value) if value == "1" => print_errors(results).map_err(From::from),
        _ => {
            let mut dirty = false;

            let stderr = std::io::stderr();
            let mut stderr = stderr.lock();
            for (path, error) in results.into_iter().filter_map(Result::err) {
                if dirty {
                    stderr.write_all(b"\n==========\n\n")?;
                }
                writeln!(
                    stderr,
                    "Error while working on {}: {:?}",
                    path.display(),
                    error
                )?;

                dirty = true;
            }

            stderr.flush().map_err(From::from)
        }
    }
}
