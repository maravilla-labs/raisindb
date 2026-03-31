# Validation

The `raisin-validation` crate centralizes validation logic for RaisinDB, providing field helpers, inheritance resolution, and schema validation that works across both server and CLI (WASM) environments.

## Overview

Validation in RaisinDB operates at multiple levels:

1. **Field-level validation** -- individual property values against their schema
2. **Type-level validation** -- node instances against their node type definitions
3. **Inheritance resolution** -- resolving field definitions across type hierarchies

## Field Helpers

The `field_helpers` module provides utilities for extracting information from `FieldSchema` variants:

```rust
use raisin_validation::field_helpers;

// Check if a field is required
let required = field_helpers::is_required(&field_schema);

// Get the field's default value
let default = field_helpers::get_default(&field_schema);

// Get validation constraints (min, max, pattern, etc.)
let constraints = field_helpers::get_constraints(&field_schema);
```

## Schema Validation

Validate field values against their schema definitions:

```rust
use raisin_validation::validate_fields;

let mut errors = Vec::new();
validate_fields(&field_schemas, &property_values, "element.fields", |err| {
    errors.push(err);
});

if !errors.is_empty() {
    for error in &errors {
        eprintln!("Validation error: {} (code: {})", error.message, error.code);
    }
}
```

## Validation Errors

Errors include structured information for consistent reporting:

```rust
use raisin_validation::ValidationError;

// Each error contains:
// - code: Machine-readable error code
// - message: Human-readable description
// - path: JSON path to the invalid field
// - severity: Error or Warning
```

## Inheritance Resolution

The inheritance system resolves field definitions across node type hierarchies. When a node type extends another, fields are merged with the child type's definitions taking precedence:

```rust
use raisin_validation::InheritanceResolver;

let resolver = InheritanceResolver::new();
let resolved_fields = resolver.resolve(&child_type, &parent_type)?;
```

This is used during node creation and updates to validate properties against the fully resolved type definition, including inherited fields from parent types.
