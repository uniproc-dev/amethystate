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

No registration call is needed. `.collect_migrations()` or `m.collect_codegen()` picks up all `#[migrate]` functions in the crate automatically.

## Versioning old structs

Old versions are defined in a submodule. The convention is `mod v1`, `mod v2`, and so on. The module is just a namespace — it does not affect storage.

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

## Manual key cleanup

When a migration involves a `ReactiveMap` field, `AmeData` holds a snapshot of its entries but does not know which raw keys to delete from the store. Pass a `MigrationContext` as a second argument to handle cleanup explicitly:

```rust
#[migrate]
fn migrate_proxy_config_v1_to_v2(
    old: AmeData<v1::ProxyConfig>,
    ctx: &mut MigrationContext,
) -> amethystate::MigrationResult<AmeData<ProxyConfig>> {
    for key in old.routes.keys() {
        ctx.delete(&format!("routes.{}", key))?;
    }

    let endpoints = old.routes
        .into_iter()
        .filter(|(k, _)| k != "obsolete")
        .map(|(k, v)| (k, ProxyEndpoint { url: v, timeout_ms: 5000 }))
        .collect();

    Ok(AmeData::<ProxyConfig> {
        name: old.name,
        endpoints,
    })
}
```

The `ctx` here is scoped to the node's prefix — `ctx.delete("routes.api")` deletes `network.routes.api`. See [Manual Migrations](./manual) for the full context API.

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