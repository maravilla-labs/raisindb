//! TypeScript bindings export test
//!
//! Run with: cargo test -p raisin-sql --features ts-export export_bindings -- --nocapture
//! Or set TS_RS_EXPORT_DIR environment variable

#[cfg(feature = "ts-export")]
mod ts_export {
    use raisin_sql::ast::ddl::{DefaultValue, IndexTypeDef, PropertyDef, PropertyTypeDef};
    use raisin_sql::ast::ddl_keywords::{DdlKeywords, KeywordCategory, KeywordInfo};
    use std::fs;
    use std::path::Path;
    use ts_rs::TS;

    #[test]
    fn export_bindings() {
        // Default to admin-console generated directory (relative to crates/raisin-sql)
        let export_dir = std::env::var("TS_RS_EXPORT_DIR")
            .unwrap_or_else(|_| "../../packages/admin-console/src/generated".to_string());

        let ddl_dir = Path::new(&export_dir).join("ddl");
        fs::create_dir_all(&ddl_dir).expect("Failed to create ddl directory");

        // Export TypeScript type definitions
        export_type::<PropertyTypeDef>(&ddl_dir);
        export_type::<IndexTypeDef>(&ddl_dir);
        export_type::<DefaultValue>(&ddl_dir);
        export_type::<PropertyDef>(&ddl_dir);
        export_type::<KeywordInfo>(&ddl_dir);
        export_type::<KeywordCategory>(&ddl_dir);
        export_type::<DdlKeywords>(&ddl_dir);

        // Export the keywords JSON data
        let keywords = DdlKeywords::all();
        let json = serde_json::to_string_pretty(&keywords).expect("Failed to serialize keywords");

        fs::write(ddl_dir.join("ddl-keywords.json"), &json)
            .expect("Failed to write ddl-keywords.json");

        println!(
            "Exported DDL keywords JSON to {}/ddl-keywords.json",
            export_dir
        );
        println!("Total keywords: {}", keywords.keywords.len());

        // Generate index.ts that re-exports all types
        generate_index_ts(&ddl_dir);
    }

    fn export_type<T: TS + 'static>(output_dir: &Path) {
        let type_name = T::name();
        let decl = T::decl();

        // Get dependencies for imports
        let deps = T::dependencies();
        let mut imports = Vec::new();
        for dep in deps {
            let dep_name = dep.ts_name;
            // Only import if it's not the same type
            if dep_name != type_name {
                imports.push(format!(
                    "import type {{ {} }} from './{}';",
                    dep_name, dep_name
                ));
            }
        }

        // Build the file content with imports and export
        let mut content = String::new();
        if !imports.is_empty() {
            content.push_str(&imports.join("\n"));
            content.push_str("\n\n");
        }

        // Add export keyword to the type declaration
        // ts-rs generates "type X = ..." so we need to add "export " prefix
        content.push_str("export ");
        content.push_str(&decl);

        let filename = format!("{}.ts", type_name);
        let filepath = output_dir.join(&filename);
        fs::write(&filepath, &content).expect(&format!("Failed to write {}", filename));
        println!("Exported {}", filename);
    }

    fn generate_index_ts(ddl_dir: &Path) {
        let index_content = r#"// Auto-generated index file for DDL types
// Re-export all generated types

export * from './PropertyTypeDef';
export * from './IndexTypeDef';
export * from './DefaultValue';
export * from './PropertyDef';
export * from './KeywordInfo';
export * from './KeywordCategory';
export * from './DdlKeywords';

// Import and re-export keywords JSON with proper typing
import ddlKeywordsJson from './ddl-keywords.json';
import type { DdlKeywords } from './DdlKeywords';

// Cast to proper type since JSON import loses type info
export const ddlKeywords = ddlKeywordsJson as DdlKeywords;
"#;

        fs::write(ddl_dir.join("index.ts"), index_content).expect("Failed to write index.ts");
        println!("Generated index.ts");
    }
}
