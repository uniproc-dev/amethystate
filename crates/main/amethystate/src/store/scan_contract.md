Every key returned is a path this library could have written, so it reads back
through [`StorePath`](crate::store::StorePath). What is listed depends on the
engine:

| engine | depth | a name no path can hold |
| --- | --- | --- |
| redb, sqlite | the whole subtree | cannot occur; a key is stored whole |
| json, toml, ron | direct children only | skipped, and logged at `warn` |

A skipped name keeps its value: it stays in the file and survives a save, but no
path addresses it, so it can be neither read nor deleted through this API. It
arrived through a text editor and leaves the same way.
