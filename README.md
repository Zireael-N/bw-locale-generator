# Usage:

    localize_npc_names <YAML FILE> <OUTPUT DIR> [MODULE NAME]

By default it'll read existing locale files and skip fetching those strings that are already there (and not commented out). To override this behaviour, set `FORCE_ALL` env variable to `1`.


## Example:

    localize_npc_names ./Examples/LittleWigs/BfA/Freehold.yaml ../LittleWigs/BfA/Freehold/Locales "Freehold Trash"


# YAML file generation:

It's sort of hacked together and not thoroughly tested, but you can give it a try:

    generate_yaml_from_one ../LittleWigs/BfA/Freehold/Trash.lua > freehold.yaml

If there are locale variables that don't have a corresponding mob ID (and vice versa), they will be printed to `stderr`.

The input file is expected to be formatted like this:

```lua
mod:RegisterEnableMob(
	1, -- NPC #1
	2 -- NPC #2
)

-- ...

if L then
	L.first = "NPC #1"
	L.second = "NPC #2"
end
```

# Bulk generation of YAML files:

```bash
# generate_yaml_from_dir <INPUT DIR> <OUTPUT DIR>
generate_yaml_from_dir ../LittleWigs ./Examples/LittleWigs
```

If `SHOW_MISSING_IDS_AND_VARS` environment variable is set to `1`, missing mob IDs and locale variables will be printed to `stderr`.

# Compilation:

- Install [Rust](https://www.rust-lang.org/);
- Run `cargo build --release`.
