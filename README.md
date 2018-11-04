# Requirements:

* Python 3;
* BeautifulSoup4;
* PyYAML

# Usage:

    ./npc_localize.py <yaml-file> <module-name>


# Example:

    ./npc_localize.py ./Examples/LittleWigs/BfA/Freehold.yaml "Freehold Trash"


# YAML file generation:

It's sort of hacked together and not thoroughly tested, but you can give it a try:

    ./generate_yaml.py ../LittleWigs/BfA/Freehold/Trash.lua > freehold.yaml

If there are locale variables that don't have a corresponding mob ID (and vice versa), they will be printed to `stderr`.

The input file is expected to be formatted like this:

```lua
mod:RegisterEnableMob(
	1, -- NPC #1
	2 -- NPC #2
)

...

if L then
	L.first = "NPC #1"
	L.second = "NPC #2"
end
```
