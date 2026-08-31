mod backends;
pub use backends::*;

use amethystate_core::{FieldExportMeta, FieldKind, SchemaExportEntry};
use heck::ToLowerCamelCase;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

pub trait FrameworkCodegen {
    fn imports(&self) -> &str {
        ""
    }
    fn extra_derives(&self) -> &[&str] {
        &[]
    }
    fn extra_attrs(&self) -> &[&str] {
        &[]
    }
}

pub struct CodegenRegistry {
    registry: BTreeMap<&'static str, &'static SchemaExportEntry>,
}

impl CodegenRegistry {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let mut registry = BTreeMap::new();
        for entry in inventory::iter::<SchemaExportEntry> {
            registry.insert(entry.struct_name, entry);
        }
        Self { registry }
    }

    pub fn export_ts(&self, out_path: impl AsRef<Path>) -> io::Result<()> {
        let mut ts = String::new();
        ts.push_str("/* eslint-disable */\n/* tslint:disable */\n// @ts-nocheck\n");
        ts.push_str(
            r#"// src/bindings/amethystate.ts DO NOT EDIT
import {invoke} from "@tauri-apps/api/core";
import {listen} from "@tauri-apps/api/event";
import { ReactiveField, ReadonlyReactiveField, ReactiveMap } from "amethystate";

"#,
        );

        let mut schema_lines = Vec::new();
        for entry in self.registry.values() {
            if let Some(prefix) = entry.prefix {
                let mut resolved = Vec::new();
                self.resolve_fields_ts(prefix, entry.fields, &mut resolved);
                for (key, ts_type, comment) in resolved {
                    if let Some(cmt) = comment {
                        schema_lines
                            .push(format!("    /** {} */\n    \"{}\": {};", cmt, key, ts_type));
                    } else {
                        schema_lines.push(format!("    \"{}\": {};", key, ts_type));
                    }
                }
            }
        }

        ts.push_str("export type StateSchema = {\n");
        for line in schema_lines {
            ts.push_str(&line);
            ts.push('\n');
        }
        ts.push_str("};\n\n");

        let mut nested_classes = String::new();
        let mut root_classes = String::new();

        for entry in self.registry.values() {
            match entry.prefix {
                None => {
                    nested_classes.push_str(&format!("export type {} = {{\n", entry.struct_name));
                    for field in entry.fields {
                        let prop_name = field.name.to_lower_camel_case();
                        let prop_type = match &field.kind {
                            FieldKind::Plain | FieldKind::Volatile => field.ts_type.to_string(),
                            FieldKind::Nested { struct_name } => struct_name.to_string(),
                            FieldKind::ReactiveMap {
                                key_type,
                                value_type,
                                ..
                            } => {
                                format!("Record<{}, {}>", key_type, value_type)
                            }
                        };
                        nested_classes.push_str(&format!("    {}: {};\n", prop_name, prop_type));
                    }
                    nested_classes.push_str("};\n\n");

                    nested_classes
                        .push_str(&format!("export class {}Fields {{\n", entry.struct_name));
                    for field in entry.fields {
                        let prop_name = field.name.to_lower_camel_case();
                        let prop_type = match &field.kind {
                            FieldKind::Plain | FieldKind::Volatile => {
                                format!("ReactiveField<{}>", field.full_ts_type)
                            }
                            FieldKind::Nested { struct_name } => format!("{}Fields", struct_name),
                            FieldKind::ReactiveMap {
                                key_type,
                                value_type,
                                ..
                            } => {
                                format!("ReactiveMap<{}, {}>", key_type, value_type)
                            }
                        };
                        nested_classes
                            .push_str(&format!("    readonly {}: {};\n", prop_name, prop_type));
                    }

                    nested_classes.push_str(
                        "    constructor(prefix: string, initialValues?: Record<string, any>) {\n",
                    );
                    for field in entry.fields {
                        let prop_name = field.name.to_lower_camel_case();
                        match &field.kind {
                            FieldKind::Plain | FieldKind::Volatile => {
                                nested_classes.push_str(&format!(
                                    "        this.{} = new ReactiveField(`${{prefix}}.{}`, initialValues?.[`${{prefix}}.{}`]);\n",
                                    prop_name, field.name, field.name
                                ));
                            }
                            FieldKind::Nested { struct_name } => {
                                nested_classes.push_str(&format!(
                                    "        this.{} = new {}Fields(`${{prefix}}.{}`, initialValues);\n",
                                    prop_name, struct_name, field.name
                                ));
                            }
                            FieldKind::ReactiveMap {
                                key_type,
                                value_type,
                                ..
                            } => {
                                nested_classes.push_str(&format!(
                                    "        this.{} = new ReactiveMap<{}, {}>(`${{prefix}}.{}`, initialValues);\n",
                                    prop_name, key_type, value_type, field.name
                                ));
                            }
                        }
                    }
                    nested_classes.push_str("    }\n}\n\n");
                }
                Some(prefix) => {
                    root_classes
                        .push_str(&format!("export type {}Plain = {{\n", entry.struct_name));
                    for field in entry.fields {
                        let prop_name = field.name.to_lower_camel_case();
                        let prop_type = match &field.kind {
                            FieldKind::Plain | FieldKind::Volatile => field.ts_type.to_string(),
                            FieldKind::Nested { struct_name } => struct_name.to_string(),
                            FieldKind::ReactiveMap {
                                key_type,
                                value_type,
                                ..
                            } => {
                                format!("Record<{}, {}>", key_type, value_type)
                            }
                        };
                        root_classes.push_str(&format!("    {}: {};\n", prop_name, prop_type));
                    }
                    root_classes.push_str("};\n\n");

                    let mut resolved = Vec::new();
                    self.resolve_fields_ts(prefix, entry.fields, &mut resolved);

                    let schema_name = format!("{}Schema", entry.struct_name);
                    root_classes.push_str(&format!("export type {} = {{\n", schema_name));
                    for (key, ts_type, comment) in &resolved {
                        if let Some(cmt) = comment {
                            root_classes.push_str(&format!(
                                "    /** {} */\n    \"{}\": {};\n",
                                cmt, key, ts_type
                            ));
                        } else {
                            root_classes.push_str(&format!("    \"{}\": {};\n", key, ts_type));
                        }
                    }
                    root_classes.push_str("};\n\n");

                    root_classes.push_str(&format!("export class {} {{\n", entry.struct_name));

                    for field in entry.fields {
                        let prop_name = field.name.to_lower_camel_case();
                        let prop_type = match &field.kind {
                            FieldKind::Plain | FieldKind::Volatile => {
                                format!("ReactiveField<{}>", field.full_ts_type)
                            }
                            FieldKind::Nested { struct_name } => format!("{}Fields", struct_name),
                            FieldKind::ReactiveMap {
                                key_type,
                                value_type,
                                ..
                            } => {
                                format!("ReactiveMap<{}, {}>", key_type, value_type)
                            }
                        };
                        root_classes
                            .push_str(&format!("    readonly {}: {};\n", prop_name, prop_type));
                    }

                    root_classes.push_str(&format!(
                        "    constructor(initialValues?: Partial<{}>) {{\n",
                        schema_name
                    ));
                    for field in entry.fields {
                        let prop_name = field.name.to_lower_camel_case();
                        let full_key = format!("{}.{}", prefix, field.name);
                        match &field.kind {
                            FieldKind::Plain | FieldKind::Volatile => {
                                root_classes.push_str(&format!(
                                    "        this.{} = new ReactiveField<{}>(\"{}\", initialValues?.[\"{}\"]);\n",
                                    prop_name, field.full_ts_type, full_key, full_key
                                ));
                            }
                            FieldKind::Nested { struct_name } => {
                                root_classes.push_str(&format!(
                                    "        this.{} = new {}Fields(\"{}\", initialValues);\n",
                                    prop_name, struct_name, full_key
                                ));
                            }
                            FieldKind::ReactiveMap {
                                key_type,
                                value_type,
                                ..
                            } => {
                                root_classes.push_str(&format!(
                                    "        this.{} = new ReactiveMap<{}, {}>(\"{}\", initialValues);\n",
                                    prop_name, key_type, value_type, full_key
                                ));
                            }
                        }
                    }
                    root_classes.push_str("    }\n\n");

                    root_classes.push_str(&format!(
                        "    static async load(): Promise<{}> {{\n",
                        entry.struct_name
                    ));
                    root_classes.push_str(&format!(
                        "        const initialValues = await invoke<Partial<{}>>(\"plugin:amethystate|amethystate_get_prefix\", {{ prefix: \"{}\" }});\n",
                        schema_name, prefix
                    ));
                    root_classes.push_str(&format!(
                        "        return new {}(initialValues);\n",
                        entry.struct_name
                    ));
                    root_classes.push_str("    }\n\n");

                    root_classes.push_str("    /**\n");
                    root_classes.push_str("     * Flushes all pending changes under this slice's prefix to disk immediately.\n");
                    root_classes.push_str("     * Resolves only when the persistent store has successfully flushed to disk.\n");
                    root_classes.push_str("     */\n");
                    root_classes.push_str("    async save(): Promise<void> {\n");
                    root_classes.push_str(&format!(
                        "        return invoke(\"plugin:amethystate|amethystate_flush\", {{ prefix: \"{}\" }});\n",
                        prefix
                    ));
                    root_classes.push_str("    }\n");

                    root_classes.push_str("}\n\n");
                }
            }
        }

        ts.push_str(&nested_classes);
        ts.push_str(&root_classes);

        if let Some(parent) = out_path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(out_path, ts)?;
        Ok(())
    }

    pub fn export_rust(
        &self,
        out_path: impl AsRef<Path>,
        fw: &dyn FrameworkCodegen,
    ) -> io::Result<()> {
        let mut code = String::new();
        code.push_str("// GENERATED AUTOMATICALLY. DO NOT EDIT.\n");
        code.push_str(fw.imports());
        code.push('\n');

        for entry in self.registry.values() {
            for attr in fw.extra_attrs() {
                code.push_str(&format!("{}\n", attr));
            }

            if let Some(prefix) = entry.prefix {
                code.push_str(&format!(
                    "#[::amethystate::amethystate(prefix = \"{}\", target = \"tauri-wasm\")]\n",
                    prefix
                ));
            } else {
                code.push_str("#[::amethystate::amethystate(target = \"tauri-wasm\")]\n");
            }

            for derive in fw.extra_derives() {
                code.push_str(&format!("#[derive({})]\n", derive));
            }

            code.push_str(&format!("pub struct {} {{\n", entry.struct_name));

            for field in entry.fields {
                let mut attributes = Vec::new();
                match &field.kind {
                    FieldKind::Volatile => attributes.push("volatile".to_string()),
                    FieldKind::Nested { .. } => attributes.push("nested".to_string()),
                    _ => {}
                }

                if !attributes.is_empty() {
                    code.push_str(&format!("    #[amestate({})]\n", attributes.join(", ")));
                }

                code.push_str(&format!("    pub {}: {},\n", field.name, field.rust_type));
            }
            code.push_str("}\n\n");
        }

        if let Some(parent) = out_path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(out_path, code)?;
        Ok(())
    }

    fn resolve_fields_ts(
        &self,
        prefix: &str,
        fields: &[FieldExportMeta],
        resolved: &mut Vec<(String, String, Option<String>)>,
    ) {
        for field in fields {
            match &field.kind {
                FieldKind::Plain => {
                    resolved.push((
                        format!("{}.{}", prefix, field.name),
                        field.full_ts_type.to_string(),
                        None,
                    ));
                }
                FieldKind::Volatile => {
                    resolved.push((
                        format!("{}.{}", prefix, field.name),
                        field.full_ts_type.to_string(),
                        Some("volatile".to_string()),
                    ));
                }
                FieldKind::Nested { struct_name } => {
                    if let Some(nested) = self.registry.get(struct_name) {
                        self.resolve_fields_ts(
                            &format!("{}.{}", prefix, field.name),
                            nested.fields,
                            resolved,
                        );
                    }
                }
                FieldKind::ReactiveMap {
                    key_type,
                    value_type,
                    ..
                } => {
                    resolved.push((
                        format!("{}.{}.[key]", prefix, field.name),
                        format!("Record<{}, {}>", key_type, value_type),
                        Some("reactive map".to_string()),
                    ));
                }
            }
        }
    }
}

#[doc = include_str!("../README.md")]
#[macro_export]
macro_rules! amethystate_codegen {
    (rs_out = $rs_out:expr, framework = $fw:ident) => {
        $crate::amethystate_codegen!(@run $rs_out, $fw, None::<&str>)
    };
    (rs_out = $rs_out:expr, framework = $fw:ident, ts_out = $ts_out:expr) => {
        $crate::amethystate_codegen!(@run $rs_out, $fw, Some($ts_out))
    };
    (ts_out = $ts_out:expr) => {
        $crate::amethystate_codegen!(@run_ts $ts_out)
    };

    (@run $rs_out:expr, $fw:ident, $ts_opt:expr) => {
        $crate::amethystate_codegen!(@exec $rs_out, $fw, $ts_opt)
    };

    (@run_ts $ts_out:expr) => {
        {
            let reg = $crate::CodegenRegistry::new();
            reg.export_ts($ts_out).expect("TS codegen failed");
        }
    };

    (@exec $rs_out:expr, dioxus, $ts_opt:expr) => {
        {
            let reg = $crate::CodegenRegistry::new();
            reg.export_rust($rs_out, &$crate::TauriDioxusCodegen).expect("Rust codegen failed");
            if let Some(ts) = $ts_opt {
                reg.export_ts(ts).expect("TS codegen failed");
            }
        }
    };

    (@exec $rs_out:expr, yew, $ts_opt:expr) => {
        {
            let reg = $crate::CodegenRegistry::new();
            reg.export_rust($rs_out, &$crate::TauriYewCodegen).expect("Rust codegen failed");
            if let Some(ts) = $ts_opt {
                reg.export_ts(ts).expect("TS codegen failed");
            }
        }
    };

    (@exec $rs_out:expr, leptos, $ts_opt:expr) => {
        {
            let reg = $crate::CodegenRegistry::new();
            reg.export_rust($rs_out, &$crate::TauriLeptosCodegen).expect("Rust codegen failed");
            if let Some(ts) = $ts_opt {
                reg.export_ts(ts).expect("TS codegen failed");
            }
        }
    };

    (@exec $rs_out:expr, vanilla, $ts_opt:expr) => {
        {
            let reg = $crate::CodegenRegistry::new();
            reg.export_rust($rs_out, &$crate::TauriVanillaCodegen).expect("Rust codegen failed");
            if let Some(ts) = $ts_opt {
                reg.export_ts(ts).expect("TS codegen failed");
            }
        }
    };
}

#[doc = include_str!("../README.md")]
#[macro_export]
macro_rules! amethystate_codegen_main {
    (rs_out = $rs_out:expr, framework = $fw:ident) => {
        fn main() {
            $crate::amethystate_codegen!(rs_out = $rs_out, framework = $fw);
        }
    };
    (ts_out = $ts_out:expr) => {
        fn main() {
            $crate::amethystate_codegen!(ts_out = $ts_out);
        }
    };
    (rs_out = $rs_out:expr, framework = $fw:ident, ts_out = $ts_out:expr) => {
        fn main() {
            $crate::amethystate_codegen!(rs_out = $rs_out, framework = $fw, ts_out = $ts_out);
        }
    };
}
