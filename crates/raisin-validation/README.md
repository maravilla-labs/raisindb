# raisin-validation

Shared validation logic for RaisinDB, providing field helpers, inheritance resolution, and schema validation that can be used by both CLI WASM and Server implementations.

## Overview

The `raisin-validation` crate centralizes validation logic that was previously duplicated across CLI and server implementations. It provides a consistent API for:

- **Field Helpers**: Utilities for extracting information from `FieldSchema` variants
- **Inheritance Resolution**: Trait-based system for resolving type hierarchies
- **Schema Validation**: Field-level and type-level validation with structured errors
- **Error Reporting**: Consistent error codes and severity levels

## Features

### Field Helpers

Helper functions for working with `FieldSchema` variants without manual matching:

```rust
use raisin_validation::{field_helpers, FieldSchema};

// Check if a field is required
let is_req = field_helpers::is_required(&field);

// Get field name from any variant
let name = field_helpers::field_name(&field);

// Check if field allows multiple values
let multiple = field_helpers::is_multiple(&field);

// Check field types
let is_section = field_helpers::is_section_field(&field);
let is_element = field_helpers::is_element_field(&field);

// Get element type name from ElementField
if let Some(type_name) = field_helpers::element_type_name(&field) {
    println!("References element type: {}", type_name);
}
```

### Schema Validation

Validate field values against their schema definitions:

```rust
use raisin_validation::{validate_fields, collect_validation_errors};
use std::collections::HashMap;

// Using callback style
let mut errors = Vec::new();
validate_fields(&fields, &values, "element.fields", |err| {
    errors.push(err);
});

// Using convenience collector
let errors = collect_validation_errors(&fields, &values, "element.fields");
for error in &errors {
    eprintln!("Validation error: {}", error);
}

// Strict mode validation (warns about extra fields)
let errors = collect_validation_errors_strict(&fields, &values, "element.fields");
```

### Inheritance Resolution

Resolve type inheritance hierarchies with circular reference detection:

```rust
use raisin_validation::{resolve_inheritance, Inheritable, TypeLoader};

// Define a type that supports inheritance
struct MyType {
    extends: Option<String>,
    fields: Vec<FieldSchema>,
    strict: Option<bool>,
}

impl Inheritable for MyType {
    fn extends(&self) -> Option<&str> {
        self.extends.as_deref()
    }

    fn fields(&self) -> &[FieldSchema] {
        &self.fields
    }

    fn strict(&self) -> Option<bool> {
        self.strict
    }
}

// Implement a loader for your storage backend
struct MyLoader { /* ... */ }

#[async_trait]
impl TypeLoader<MyType> for MyLoader {
    async fn load(&self, name: &str) -> Result<Option<MyType>> {
        // Load from your storage
    }
}

// Resolve the complete inheritance hierarchy
let resolved = resolve_inheritance(my_type, "MyType", &loader).await?;

// Access resolved fields (with inheritance flattened)
for field in &resolved.resolved_fields {
    println!("Field: {}", field_helpers::field_name(field));
}

// Check inheritance chain
println!("Inherits from: {:?}", resolved.inheritance_chain);
```

### Structured Errors

All validation errors use structured types with consistent error codes:

```rust
use raisin_validation::{ValidationError, Severity, codes};

// Create validation errors
let err = ValidationError::missing_required("element.fields", "title");
let err = ValidationError::unknown_element_type("element.fields.content", "Article");
let err = ValidationError::circular_inheritance("Article", &["Base", "Article"]);
let err = ValidationError::max_inheritance_depth("Article", 20);
let err = ValidationError::strict_mode_violation("element.fields", "extra_field");
let err = ValidationError::invalid_field_value("element.fields.age", "age", "must be positive");

// Check error properties
match err.severity {
    Severity::Error => println!("Critical error"),
    Severity::Warning => println!("Warning"),
}

println!("Error code: {}", err.code);
println!("Path: {}", err.path);
println!("Message: {}", err.message);

// Convert to raisin_error::Error for use with Result
let raisin_err: raisin_error::Error = err.into();
```

## Error Codes

The following standardized error codes are provided:

- `MISSING_REQUIRED_FIELD` - A required field is missing
- `MISSING_REQUIRED_ELEMENT_FIELD` - Required field on element type missing
- `MISSING_REQUIRED_ARCHETYPE_FIELD` - Required field on archetype missing
- `UNKNOWN_ELEMENT_TYPE` - Referenced element type does not exist
- `CIRCULAR_INHERITANCE` - Circular inheritance detected
- `STRICT_MODE_VIOLATION` - Unexpected field in strict mode
- `MAX_INHERITANCE_DEPTH` - Maximum inheritance depth exceeded
- `INVALID_FIELD_VALUE` - Invalid field value type or format

## Architecture

### Field Helpers Module (`field_helpers`)

Provides zero-cost abstractions for working with `FieldSchema` variants. All functions are simple match expressions that compile down to efficient code.

### Inheritance Module (`inheritance`)

Implements the `Inheritable` trait and `TypeLoader` trait for dependency injection. The `resolve_inheritance` function walks the inheritance chain, detecting:

- Circular references (using a visited set)
- Depth limit violations (max 20 levels)
- Missing parent types
- Field override behavior (child fields override parent fields)
- Strict mode inheritance (inherits from parent if not set)

### Schema Module (`schema`)

Provides validation functions with callback-based error reporting. This allows:

- Flexible error handling (collect, early return, logging, etc.)
- Zero allocations when using callbacks
- Convenience functions for common patterns

### Errors Module (`errors`)

Defines structured error types with:

- Machine-readable error codes
- Human-readable messages
- Path information for nested validation
- Severity levels (Error vs Warning)
- Conversion to `raisin_error::Error`

## Design Principles

1. **No Storage Dependencies**: The crate uses traits to abstract storage access, making it usable in WASM, server, and test contexts

2. **Callback-Based Validation**: Validation functions use callbacks to report errors, allowing flexible error handling without allocations

3. **Type Safety**: Leverages Rust's type system to prevent common errors (e.g., using `Option<&str>` for extends to prevent null references)

4. **Zero-Cost Abstractions**: Helper functions compile to the same code as manual matching

5. **Comprehensive Testing**: Each module includes unit tests demonstrating correct behavior

## Usage in Different Contexts

### CLI WASM

```rust
// Use in-memory type loader
struct WasmTypeLoader {
    types: HashMap<String, ElementType>,
}

// Validate before committing
let errors = collect_validation_errors(&fields, &values, "");
if !errors.is_empty() {
    return Err(format!("Validation failed: {:?}", errors));
}
```

### Server

```rust
// Use database-backed type loader
struct DbTypeLoader<'a> {
    db: &'a Database,
}

#[async_trait]
impl TypeLoader<ElementType> for DbTypeLoader<'_> {
    async fn load(&self, name: &str) -> Result<Option<ElementType>> {
        self.db.get_element_type(name).await
    }
}

// Validate incoming requests
let resolved = resolve_inheritance(element_type, &name, &loader).await?;
validate_fields(&resolved.resolved_fields, &request.data, "data", |err| {
    log::warn!("Validation error: {}", err);
});
```

## Dependencies

- `raisin-models` - Field schema and property value types
- `raisin-error` - Common error types
- `async-trait` - Async trait support for TypeLoader
- `thiserror` - Error derive macros

## License

Business Source License 1.1 (BSL-1.1)
