---
title: Defining Migration Steps
sidebar:
  order: 21
---


## The #[migrate] macro

`#[migrate]` transforms a plain function into a registered migration step. The macro derives everything it needs from the function signature:

- the source version from the argument type (`AmeData<v1::Config>`)
- the target version from the return type (`AmeData<Config>`)
- the step description from the function name

```rust
#[migrate]
fn migrate_config_v1_to_v2(old: AmeData<v1::Config>) -> amethystate::MigrationResult<AmeData<Config>> {
    Ok(AmeData::<Config> {
        address: old.host,
        port: old.port,
    })
}
```

No registration call is needed. `build_with_migration` picks up every `#[migrate]` function in the binary — that is the linker's answer rather than a list anyone keeps. To collect them into a builder you are assembling by hand, `m.collect_codegen()` does the same thing.

## Versioning old structs

A step takes the old struct as its argument, so the old struct has to still exist as a type. Where you put it is your business — nothing here reads the module path, and storage does not depend on it. The examples use `mod v1` because the old struct and the new one usually want the same name.

```rust
mod v1 {
    use super::*;

    #[amethystate(prefix = "app", version = 1)]
    pub struct Config {
        #[amestate(default = "localhost".to_string())]
        pub host: String,

        #[amestate(default = 8080)]
        pub port: u16,
    }
}

#[amethystate(prefix = "app", version = 2)]
pub struct Config {
    #[amestate(default = "localhost".to_string())]
    pub address: String,

    #[amestate(default = 8080)]
    pub port: u16,
}
```

Both structs share the same `prefix`. The version number is what the migrator uses to determine which step to run.

## AmeData

`AmeData<T>` is a plain data container that mirrors the fields of an `#[amethystate]` struct without reactive wrappers. It is what you read from and write to inside a migration step.

Fields are accessed directly:

```rust
fn migrate_config_v1_to_v2(old: AmeData<v1::Config>) -> amethystate::MigrationResult<AmeData<Config>> {
    Ok(AmeData::<Config> {
        address: old.host, // direct field access
        port: old.port,
    })
}
```

## Declaring renames

`#[rename(old_field => new_field)]` declares that a field was renamed between versions. It can be stacked for multiple renames. Each `#[rename]` generates a compile-time check that both fields exist on the respective types — a typo is a compile error:

```rust
#[migrate]
#[rename(login => username, tier => plan)]
fn migrate_identity_v1_to_v2(
    old: AmeData<v1::Identity>,
) -> amethystate::MigrationResult<AmeData<Identity>> {
    Ok(AmeData::<Identity> {
        username: old.login,
        plan: match old.tier.as_str() {
            "pro" => "professional".to_string(),
            other => other.to_string(),
        },
        created_at_ms: 1_700_000_000_000,
    })
}
```

`#[rename]` is a declaration, not an implementation. The actual field mapping is still written by hand in the function body. The attribute exists to produce the compile-time check and to record the rename in the schema history.

## Cleanup the step already does

Every generated step ends by removing each place the old declaration owned and the new one does not. A map is a place and owns everything under it — see [Who owns which place](/amethystate/concepts/claims/) — so a map that was dropped or renamed takes its entries with it. There is nothing to loop over and nothing to write.

The other case is a map that stays while an entry leaves it. `routes` is declared in both versions, but the `obsolete` key should not be there any more. The new map is written entry by entry: the ones it holds are written over, and `routes.obsolete` is not touched — it stays on disk. That is what a `MigrationContext` second argument is for:

```rust
#[migrate]
fn migrate_proxy_config_v1_to_v2(
    old: AmeData<v1::ProxyConfig>,
    ctx: &mut MigrationContext,
) -> amethystate::MigrationResult<AmeData<ProxyConfig>> {
    ctx.delete("routes.obsolete")?;

    let routes = old
        .routes
        .into_iter()
        .filter(|(key, _)| key != "obsolete")
        .collect();

    Ok(AmeData::<ProxyConfig> {
        name: old.name,
        routes,
    })
}
```

The `ctx` is scoped to the node's prefix — `ctx.delete("routes.api")` deletes `network.routes.api`, and `ctx.delete_prefix("routes")` takes the level and everything under it in one call. See [Manual Migrations](/amethystate/migrations/manual/) for the full context API.

## Multi-step paths

If a prefix goes through more than one version, define a step for each transition. The migrator chains them in order:

```rust
// v1 → v2
#[migrate]
#[rename(title => name)]
fn migrate_workspace_v1_to_v2(
    old: AmeData<workspace_v1::Workspace>,
) -> amethystate::MigrationResult<AmeData<workspace_v2::Workspace>> {
    Ok(AmeData::<workspace_v2::Workspace> {
        name: old.title,
        appearance_theme: old.theme,
    })
}

// v2 → v3
#[migrate]
fn migrate_workspace_v2_to_v3(
    old: AmeData<workspace_v2::Workspace>,
) -> amethystate::MigrationResult<AmeData<Workspace>> {
    Ok(AmeData::<Workspace> {
        name: old.name,
        appearance_theme: old.appearance_theme,
        welcome_title: "Welcome".to_string(),
    })
}
```

Each step only needs to know about the version immediately before it.