use indexmap::IndexMap as Map;
use std::{
    env,
    fs::{self, File},
    io::BufReader,
    path::PathBuf,
};

use localize_npc_names::{Error, Localizer};

#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

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

    let ids_map: Map<String, i64> =
        serde_yaml::from_reader(BufReader::new(File::open(&yaml_path)?))?;
    let force_all = match env::var_os("FORCE_ALL") {
        Some(ref v) if v == "1" => true,
        _ => false,
    };
    let module_name = match module_name {
        Some(module_name) => module_name,
        None => format!("{} Trash", yaml_path.file_stem().unwrap().to_string_lossy()),
    };

    fs::create_dir_all(&output_dir)?;
    Localizer::run(ids_map, &module_name, output_dir, force_all);

    Ok(())
}
