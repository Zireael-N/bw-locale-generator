use indexmap::IndexMap as Map;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::{
    env, fmt,
    fs::File,
    io::{self, BufRead, BufReader, Write},
};

#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Serialize)]
struct ParseResult {
    module_name: Option<String>,
    #[serde(rename = "npcs")]
    var_to_id_map: Map<String, i64>,
    #[serde(skip)]
    missing_vars: Vec<(String, String)>,
    #[serde(skip)]
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

fn parse(mut file: BufReader<File>) -> Result<ParseResult, io::Error> {
    static IDS_START: &str = "mod:RegisterEnableMob(";
    static VARS_START: &str = "if L then";

    static ID_REGEX: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"^\s*(\d+),?\s*--\s*(.+?)\n?$"#).unwrap());
    static VAR_REGEX: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"^\s*L\.(\w+)\s*=\s*"(.+)""#).unwrap());
    static MODULE_DECL_REGEX: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"^\s*local\s*mod(?:,\s*CL)?\s*=\s*BigWigs:NewBoss\("(.*?)""#).unwrap()
    });

    let mut ids_map = Map::with_capacity(16);
    let mut vars_map = Map::with_capacity(16);
    let mut module_name = None;

    let mut state = ParseState::Neither;
    let mut parsed_blocks = 0;

    let mut line = String::new();
    while file.read_line(&mut line)? > 0 {
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
                        if parsed_blocks == 1 {
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
                    if line.trim() == "end" {
                        state = ParseState::Neither;
                        if parsed_blocks == 1 {
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
                }
            }
        }
        line.clear();
    }

    let mut var_to_id_map = Map::with_capacity(vars_map.len());
    let mut missing_vars = Vec::with_capacity(4);

    for (value, variable) in vars_map.into_iter() {
        if let Some(id) = ids_map.shift_remove(&value) {
            var_to_id_map.insert(variable, id);
        } else {
            missing_vars.push((variable, value));
        }
    }

    let missing_ids: Vec<_> = ids_map
        .into_iter()
        .map(|(comment, id)| (id, comment))
        .collect();

    Ok(ParseResult {
        module_name,
        var_to_id_map,
        missing_vars,
        missing_ids,
    })
}

fn pretty_print(parse_result: ParseResult) -> Result<(), io::Error> {
    let mut stdout = io::stdout().lock();
    let serialized = toml::to_string_pretty(&parse_result).map_err(io::Error::other)?;
    stdout.write_all(serialized.as_bytes())?;
    stdout.flush()?;
    drop(stdout);

    let mut stderr = io::stderr().lock();

    if !parse_result.missing_vars.is_empty() {
        stderr.write_all(b"\nMissing variables:\n")?;
        for (variable, value) in parse_result.missing_vars.iter() {
            writeln!(stderr, "{variable} (\"{value}\")")?;
        }
    }

    if !parse_result.missing_ids.is_empty() {
        stderr.write_all(b"\nMissing IDs:\n")?;
        for (id, comment) in parse_result.missing_ids.iter() {
            writeln!(stderr, "{id} (\"{comment}\")")?;
        }
    }

    Ok(())
}

fn main() -> Result<(), Error> {
    let filename = {
        let mut args = env::args_os();
        let program_name = args.next().unwrap();

        if let Some(filename) = args.next() {
            filename
        } else {
            eprintln!("Usage: {} module.lua", program_name.to_string_lossy());
            std::process::exit(1);
        }
    };

    let file = File::open(&filename)?;
    match file.metadata() {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err(format!("{} is not a valid file", filename.to_string_lossy()).into());
            }
        }
        Err(err) => return Err(err.into()),
    }

    let result = parse(BufReader::new(file))?;

    pretty_print(result).map_err(From::from)
}
