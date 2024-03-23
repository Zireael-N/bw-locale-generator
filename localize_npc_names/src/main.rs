use indexmap::IndexMap as Map;
use serde::Deserialize;
use std::{
    env,
    fs::{self, File},
    io::{BufReader, Seek, SeekFrom},
    path::PathBuf,
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
    let (yaml_path, output_dir, module_name) = {
        let mut args = env::args_os();
        let program_name = args.next().unwrap();

        match (
            args.next(),
            args.next(),
            args.next()
                .map(|value| value.to_string_lossy().into_owned()),
        ) {
            (Some(yaml_path), Some(output_dir), module_name) => (
                PathBuf::from(yaml_path),
                PathBuf::from(output_dir),
                module_name,
            ),
            (_, _, _) => {
                eprintln!(
                    "Usage: {} <YAML FILE> <OUTPUT DIR> [MODULE NAME]",
                    program_name.to_string_lossy()
                );
                std::process::exit(1);
            }
        }
    };

    let force_all = matches!(env::var_os("FORCE_ALL"), Some(ref v) if v == "1");
    let mut input_file = BufReader::new(File::open(&yaml_path)?);
    let (ids_map, module_name) = match serde_yaml::from_reader::<_, InputFile>(&mut input_file) {
        Ok(input) => {
            let module_name = match (module_name, input.module_name) {
                (Some(inner), _) => inner,
                (_, Some(inner)) => inner,
                _ => {
                    eprintln!(
                        "WARNING: module_name is missing, falling back to using the file's name"
                    );
                    format!(
                        "{} Trash",
                        &yaml_path.file_stem().unwrap().to_string_lossy()
                    )
                }
            };
            (input.npcs, module_name)
        }
        Err(_) => {
            input_file.seek(SeekFrom::Start(0))?;
            let ids_map = serde_yaml::from_reader(&mut input_file)?;
            let module_name = match module_name {
                Some(module_name) => module_name,
                None => format!(
                    "{} Trash",
                    &yaml_path.file_stem().unwrap().to_string_lossy()
                ),
            };
            (ids_map, module_name)
        }
    };

    fs::create_dir_all(&output_dir)?;
    Localizer::run(ids_map, &module_name, output_dir, force_all);

    Ok(())
}
