use crate::analyzer::types::DataType;

use super::types::{FunctionCategory, FunctionRegistry, FunctionSignature};

/// Register system, geospatial, auth, and invocation built-in functions.
pub(super) fn register(registry: &mut FunctionRegistry) {
    register_system(registry);
    register_geospatial(registry);
    register_auth(registry);
    register_invoke(registry);
}

/// System information functions (PostgreSQL compatibility).
fn register_system(registry: &mut FunctionRegistry) {
    registry.register(FunctionSignature {
        name: "VERSION".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "CURRENT_SCHEMA".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "CURRENT_DATABASE".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "CURRENT_USER".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "SESSION_USER".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "CURRENT_CATALOG".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });
}

/// Geospatial functions (PostGIS-compatible).
fn register_geospatial(registry: &mut FunctionRegistry) {
    // ST_POINT - Create a Point geometry from longitude and latitude
    registry.register(FunctionSignature {
        name: "ST_POINT".into(),
        params: vec![DataType::Double, DataType::Double],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_GEOMFROMGEOJSON".into(),
        params: vec![DataType::Text],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_ASGEOJSON".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_DISTANCE".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_DWITHIN".into(),
        params: vec![DataType::Geometry, DataType::Geometry, DataType::Double],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_CONTAINS".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_WITHIN".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_INTERSECTS".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_X".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_Y".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });
}

/// Authentication configuration functions (RAISIN_AUTH_*).
fn register_auth(registry: &mut FunctionRegistry) {
    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_CURRENT_USER".into(),
        params: vec![],
        return_type: DataType::Nullable(Box::new(DataType::Text)),
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_CURRENT_USER".into(),
        params: vec![],
        return_type: DataType::Nullable(Box::new(DataType::JsonB)),
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_CURRENT_WORKSPACE".into(),
        params: vec![],
        return_type: DataType::Nullable(Box::new(DataType::Text)),
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_HAS_PERMISSION".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_GET_SETTINGS".into(),
        params: vec![],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_UPDATE_SETTINGS".into(),
        params: vec![DataType::Text],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_ADD_PROVIDER".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_UPDATE_PROVIDER".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_REMOVE_PROVIDER".into(),
        params: vec![DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });
}

/// Function invocation from SQL: INVOKE() and INVOKE_SYNC().
fn register_invoke(registry: &mut FunctionRegistry) {
    for name in ["INVOKE", "INVOKE_SYNC"] {
        // 1-arg: INVOKE(path)
        registry.register(FunctionSignature {
            name: name.into(),
            params: vec![DataType::Text],
            return_type: DataType::JsonB,
            is_deterministic: false,
            category: FunctionCategory::System,
        });
        // 2-arg: INVOKE(path, input)
        registry.register(FunctionSignature {
            name: name.into(),
            params: vec![DataType::Text, DataType::JsonB],
            return_type: DataType::JsonB,
            is_deterministic: false,
            category: FunctionCategory::System,
        });
        // 3-arg: INVOKE(path, input, workspace)
        registry.register(FunctionSignature {
            name: name.into(),
            params: vec![DataType::Text, DataType::JsonB, DataType::Text],
            return_type: DataType::JsonB,
            is_deterministic: false,
            category: FunctionCategory::System,
        });
    }
}
