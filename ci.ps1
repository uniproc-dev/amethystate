$ErrorActionPreference = "Stop"
$env:INSTA_UPDATE = "no"

function Exec {
    param ([scriptblock]$ScriptBlock)
    & $ScriptBlock
    if ($LASTEXITCODE -ne 0) {
        Remove-Item Env:\INSTA_UPDATE -ErrorAction SilentlyContinue
        exit $LASTEXITCODE
    }
}

Exec { cargo fmt --all --check }
Exec { cargo clippy --workspace --all-targets --no-default-features --features redb -- -D warnings }
Exec { cargo clippy --workspace --all-targets --no-default-features --features json -- -D warnings }
Exec { cargo clippy --workspace --all-targets --no-default-features --features toml -- -D warnings }
Exec { cargo clippy --workspace --all-targets --no-default-features --features ron -- -D warnings }
Exec { cargo clippy --workspace --all-targets --no-default-features --features sqlite -- -D warnings }
Exec { cargo clippy --workspace --all-targets --all-features -- -D warnings }

Exec { cargo test --workspace --no-default-features --features redb }
Exec { cargo test --workspace --no-default-features --features json }
Exec { cargo test --workspace --no-default-features --features toml }
Exec { cargo test --workspace --no-default-features --features ron }
Exec { cargo test --workspace --no-default-features --features sqlite }
Exec { cargo test --workspace --all-features }
#
#$examples = Get-ChildItem -Path "examples" -Directory
#foreach ($example in $examples) {
#    Push-Location $example.FullName
#    Exec { cargo build }
#    Pop-Location
#}
#
#$wasmCrates = @(
#    "crates\adapters\amethystate-dioxus",
#    "crates\adapters\amethystate-leptos",
#    "crates\adapters\amethystate-yew",
#    "crates\adapters\amethystate-tauri",
#    "crates\main\amethystate-arena"
#)
#
#foreach ($crate in $wasmCrates) {
#    Push-Location $crate
#    Exec { cargo build --target wasm32-unknown-unknown }
#    Pop-Location
#}