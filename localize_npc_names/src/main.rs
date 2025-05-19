use indexmap::IndexMap as Map;
use serde::Deserialize;
use std::{
    borrow::Cow,
    env, fs,
    path::{Path, PathBuf},
};

use localize_npc_names::{Error, Localizer};

#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[derive(Debug, Deserialize)]
struct InputFile {
    module_name: Option<String>,
    npcs: Map<String, i64>,
}

fn main() -> Result<(), Error> {
    let (input_file_path, output_dir, module_name) = {
        let mut args = env::args_os();
        let program_name = args.next().unwrap();

        match (
            args.next(),
            args.next(),
            args.next()
                .map(|value| value.to_string_lossy().into_owned()),
        ) {
            (Some(input_file_path), Some(output_dir), module_name) => (
                PathBuf::from(input_file_path),
                PathBuf::from(output_dir),
                module_name,
            ),
            (_, _, _) => {
                eprintln!(
                    "Usage: {} <TOML FILE> <OUTPUT DIR> [MODULE NAME]",
                    program_name.to_string_lossy()
                );
                std::process::exit(1);
            }
        }
    };

    let force_all = matches!(env::var_os("FORCE_ALL"), Some(ref v) if v == "1");
    let input_file = fs::read_to_string(&input_file_path)?;
    let (ids_map, module_name) = match toml::from_str::<InputFile>(&input_file) {
        Ok(input) => {
            let module_name = Cow::Owned(match (module_name, input.module_name) {
                (Some(inner), _) => inner,
                (_, Some(inner)) => inner,
                _ => {
                    panic!("TOML file does not contain module_name");
                }
            });

            (input.npcs, module_name)
        }
        Err(_) => parse_yaml(&input_file_path, module_name.as_deref(), &input_file)?,
    };

    fs::create_dir_all(&output_dir)?;
    Localizer::run(ids_map, &module_name, output_dir, force_all);

    Ok(())
}

fn parse_yaml<'m>(
    file_path: &Path,
    module_name: Option<&'m str>,
    file_contents: &str,
) -> Result<(Map<String, i64>, Cow<'m, str>), Error> {
    match serde_yaml::from_str::<InputFile>(file_contents) {
        Ok(input) => {
            let module_name = match (module_name, input.module_name) {
                (Some(inner), _) => Cow::Borrowed(inner),
                (_, Some(inner)) => Cow::Owned(inner),
                _ => {
                    eprintln!(
                        "WARNING: module_name is missing, falling back to using the file's name"
                    );
                    Cow::Owned(format!(
                        "{} Trash",
                        &file_path.file_stem().unwrap().to_string_lossy()
                    ))
                }
            };

            eprintln!("WARNING: YAML support is being phased out, use TOML instead");
            Ok((input.npcs, module_name))
        }
        Err(_) => {
            let ids_map = serde_yaml::from_str(file_contents)?;
            let module_name = match module_name {
                Some(module_name) => Cow::Borrowed(module_name),
                None => Cow::Owned(format!(
                    "{} Trash",
                    &file_path.file_stem().unwrap().to_string_lossy()
                )),
            };

            eprintln!("WARNING: YAML support is being phased out, use TOML instead");
            Ok((ids_map, module_name))
        }
    }
}
