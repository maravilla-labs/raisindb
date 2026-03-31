use std::fmt;

/// SQL data types supported by RaisinDB
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataType {
    // Numeric types
    Int,    // 32-bit integer
    BigInt, // 64-bit integer
    Double, // 64-bit float (maps to PropertyValue::Float)

    // Text types
    Boolean,
    Text, // UTF-8 string
    Uuid, // UUID strings

    // Temporal types
    TimestampTz, // Timestamp with timezone (UTC normalized)
    Interval,    // Time interval/duration

    // RaisinDB-specific types
    Path,          // Hierarchical path (e.g., "/content/blog/post1")
    JsonB,         // JSON data (maps to PropertyValue and nested structures)
    Vector(usize), // Fixed-dimension vector for embeddings
    Geometry,      // GeoJSON geometry (Point, LineString, Polygon, etc.)

    // Full-text search types (PostgreSQL-style)
    TSVector, // Full-text search document (tsvector in PostgreSQL)
    TSQuery,  // Full-text search query (tsquery in PostgreSQL)

    // Collection types (PostgreSQL-style)
    Array(Box<DataType>), // Array of elements of a specific type (e.g., TEXT[], INT[])

    // Nullable wrapper
    Nullable(Box<DataType>),

    // Unknown (used during inference)
    Unknown,
}

impl DataType {
    /// Check if this type can be implicitly coerced to another type
    pub fn can_coerce_to(&self, target: &DataType) -> bool {
        // Exact match
        if self == target {
            return true;
        }

        // Unwrap nullable types for comparison
        let (self_base, target_base) = match (self, target) {
            (DataType::Nullable(from), DataType::Nullable(to)) => (from.as_ref(), to.as_ref()),
            (from, DataType::Nullable(to)) => (from, to.as_ref()),
            (DataType::Nullable(from), to) => (from.as_ref(), to),
            (from, to) => (from, to),
        };

        // Base types match after unwrapping nullability — always coercible.
        // Covers: T → Nullable(T), Nullable(T) → T, Nullable(T) → Nullable(T)
        // Functions handle NULL at runtime (like PostgreSQL).
        if self_base == target_base {
            return true;
        }

        // Numeric ladder: INT → BIGINT → DOUBLE
        match (self_base, target_base) {
            (DataType::Int, DataType::BigInt) => true,
            (DataType::Int, DataType::Double) => true,
            (DataType::BigInt, DataType::Double) => true,

            // PATH can be compared with TEXT literals (TEXT → PATH)
            (DataType::Text, DataType::Path) => true,

            // Unknown can coerce to anything (for inference)
            (DataType::Unknown, _) => true,
            (_, DataType::Unknown) => true,

            _ => false,
        }
    }

    /// Check if this type can be explicitly cast to another type (more permissive than coercion)
    /// Used for CAST operations which may fail at runtime but are allowed in SQL
    pub fn can_cast_to(&self, target: &DataType) -> bool {
        // First check if implicit coercion is allowed
        if self.can_coerce_to(target) {
            return true;
        }

        // Unwrap nullable types for casting
        let (self_base, target_base) = match (self, target) {
            (DataType::Nullable(from), DataType::Nullable(to)) => (from.as_ref(), to.as_ref()),
            (from, DataType::Nullable(to)) => (from, to.as_ref()),
            (DataType::Nullable(from), to) => (from.as_ref(), to),
            (from, to) => (from, to),
        };

        // Explicit casts between TEXT and other types
        match (self_base, target_base) {
            // TEXT ↔ numeric conversions (may fail at runtime)
            (DataType::Text, DataType::Int) => true,
            (DataType::Text, DataType::BigInt) => true,
            (DataType::Text, DataType::Double) => true,
            (DataType::Int, DataType::Text) => true,
            (DataType::BigInt, DataType::Text) => true,
            (DataType::Double, DataType::Text) => true,
            (DataType::Boolean, DataType::Text) => true,
            (DataType::Text, DataType::Boolean) => true,

            // Numeric downcasts (lossy, but allowed in explicit cast)
            (DataType::Double, DataType::Int) => true,
            (DataType::Double, DataType::BigInt) => true,
            (DataType::BigInt, DataType::Int) => true,

            // PATH ↔ TEXT bidirectional (explicit cast)
            (DataType::Path, DataType::Text) => true,

            // JSONB ↔ TEXT conversions
            (DataType::JsonB, DataType::Text) => true,
            (DataType::Text, DataType::JsonB) => true,

            // GEOMETRY ↔ TEXT conversions (GeoJSON format)
            (DataType::Geometry, DataType::Text) => true,
            (DataType::Text, DataType::Geometry) => true,

            // TIMESTAMPTZ ↔ TEXT conversions (ISO 8601 format)
            (DataType::TimestampTz, DataType::Text) => true,
            (DataType::Text, DataType::TimestampTz) => true,

            _ => false,
        }
    }

    /// Get the common type for binary operations
    /// Returns None if types are incompatible
    pub fn common_type(&self, other: &DataType) -> Option<DataType> {
        // Exact match
        if self == other {
            return Some(self.clone());
        }

        // Unwrap nullable types
        let (self_base, other_base) = match (self, other) {
            (DataType::Nullable(a), DataType::Nullable(b)) => (a.as_ref(), b.as_ref()),
            (DataType::Nullable(a), b) => (a.as_ref(), b),
            (a, DataType::Nullable(b)) => (a, b.as_ref()),
            (a, b) => (a, b),
        };

        // Determine if result should be nullable
        let is_nullable =
            matches!(self, DataType::Nullable(_)) || matches!(other, DataType::Nullable(_));

        // Numeric ladder
        let common = match (self_base, other_base) {
            // Same base type
            (a, b) if a == b => Some(a.clone()),

            // Numeric promotions
            (DataType::Int, DataType::BigInt) | (DataType::BigInt, DataType::Int) => {
                Some(DataType::BigInt)
            }
            (DataType::Int, DataType::Double) | (DataType::Double, DataType::Int) => {
                Some(DataType::Double)
            }
            (DataType::BigInt, DataType::Double) | (DataType::Double, DataType::BigInt) => {
                Some(DataType::Double)
            }

            // PATH and TEXT are compatible
            (DataType::Path, DataType::Text) | (DataType::Text, DataType::Path) => {
                Some(DataType::Path)
            }

            // Unknown propagates
            (DataType::Unknown, other) | (other, DataType::Unknown) => Some(other.clone()),

            // No other implicit conversions
            _ => None,
        }?;

        // Wrap in nullable if either operand was nullable
        if is_nullable {
            Some(common.as_nullable())
        } else {
            Some(common)
        }
    }

    /// Check if type is nullable
    pub fn is_nullable(&self) -> bool {
        matches!(self, DataType::Nullable(_))
    }

    /// Make this type nullable
    pub fn as_nullable(self) -> DataType {
        match self {
            DataType::Nullable(_) => self,
            other => DataType::Nullable(Box::new(other)),
        }
    }

    /// Get the base type (unwrap nullable)
    pub fn base_type(&self) -> &DataType {
        match self {
            DataType::Nullable(inner) => inner.as_ref(),
            other => other,
        }
    }

    /// Get an intermediate type for two-step casting.
    /// Returns Some(intermediate_type) if source can be cast to target via intermediate.
    /// Returns None if direct cast is possible or no intermediate path exists.
    ///
    /// This enables casts like JSONB → TEXT → BOOLEAN where direct JSONB → BOOLEAN
    /// is not allowed, but the intermediate path through TEXT works.
    pub fn get_intermediate_cast_type(&self, target: &DataType) -> Option<DataType> {
        // If direct cast works, no intermediate needed
        if self.can_cast_to(target) {
            return None;
        }

        // Unwrap nullable types for checking
        let (self_base, target_base) = match (self, target) {
            (DataType::Nullable(from), DataType::Nullable(to)) => (from.as_ref(), to.as_ref()),
            (from, DataType::Nullable(to)) => (from, to.as_ref()),
            (DataType::Nullable(from), to) => (from.as_ref(), to),
            (from, to) => (from, to),
        };

        // JSONB can cast to TEXT, check if TEXT can cast/coerce to target
        if matches!(self_base, DataType::JsonB) {
            // TEXT can cast to these types (from can_cast_to and can_coerce_to)
            if matches!(
                target_base,
                DataType::Boolean
                    | DataType::Int
                    | DataType::BigInt
                    | DataType::Double
                    | DataType::Path
            ) {
                return Some(DataType::Text);
            }
        }

        None
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataType::Int => write!(f, "INT"),
            DataType::BigInt => write!(f, "BIGINT"),
            DataType::Double => write!(f, "DOUBLE"),
            DataType::Boolean => write!(f, "BOOLEAN"),
            DataType::Text => write!(f, "TEXT"),
            DataType::Uuid => write!(f, "UUID"),
            DataType::TimestampTz => write!(f, "TIMESTAMPTZ"),
            DataType::Interval => write!(f, "INTERVAL"),
            DataType::Path => write!(f, "PATH"),
            DataType::JsonB => write!(f, "JSONB"),
            DataType::Vector(dim) => write!(f, "VECTOR({})", dim),
            DataType::Geometry => write!(f, "GEOMETRY"),
            DataType::TSVector => write!(f, "TSVECTOR"),
            DataType::TSQuery => write!(f, "TSQUERY"),
            DataType::Array(elem_type) => write!(f, "{}[]", elem_type),
            DataType::Nullable(inner) => write!(f, "{}?", inner),
            DataType::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match_coercion() {
        assert!(DataType::Int.can_coerce_to(&DataType::Int));
        assert!(DataType::Text.can_coerce_to(&DataType::Text));
        assert!(DataType::Path.can_coerce_to(&DataType::Path));
    }

    #[test]
    fn test_numeric_ladder_coercion() {
        // INT → BIGINT
        assert!(DataType::Int.can_coerce_to(&DataType::BigInt));
        assert!(!DataType::BigInt.can_coerce_to(&DataType::Int));

        // INT → DOUBLE
        assert!(DataType::Int.can_coerce_to(&DataType::Double));
        assert!(!DataType::Double.can_coerce_to(&DataType::Int));

        // BIGINT → DOUBLE
        assert!(DataType::BigInt.can_coerce_to(&DataType::Double));
        assert!(!DataType::Double.can_coerce_to(&DataType::BigInt));
    }

    #[test]
    fn test_nullable_coercion() {
        // Non-nullable → Nullable
        assert!(DataType::Int.can_coerce_to(&DataType::Nullable(Box::new(DataType::Int))));
        assert!(DataType::Text.can_coerce_to(&DataType::Nullable(Box::new(DataType::Text))));

        // With numeric ladder
        assert!(DataType::Int.can_coerce_to(&DataType::Nullable(Box::new(DataType::BigInt))));

        // Nullable → Non-nullable (functions handle NULL at runtime)
        assert!(DataType::Nullable(Box::new(DataType::Int)).can_coerce_to(&DataType::Int));
        assert!(DataType::Nullable(Box::new(DataType::Text)).can_coerce_to(&DataType::Text));
        assert!(DataType::Nullable(Box::new(DataType::JsonB)).can_coerce_to(&DataType::JsonB));
        assert!(DataType::Nullable(Box::new(DataType::Boolean)).can_coerce_to(&DataType::Boolean));

        // Nullable with numeric ladder
        assert!(DataType::Nullable(Box::new(DataType::Int)).can_coerce_to(&DataType::BigInt));
        assert!(DataType::Nullable(Box::new(DataType::Int)).can_coerce_to(&DataType::Double));

        // Incompatible types still fail even with nullable
        assert!(!DataType::Nullable(Box::new(DataType::Text)).can_coerce_to(&DataType::Int));
        assert!(!DataType::Nullable(Box::new(DataType::JsonB)).can_coerce_to(&DataType::Text));
    }

    #[test]
    fn test_path_text_coercion() {
        // TEXT → PATH (for literal comparisons)
        assert!(DataType::Text.can_coerce_to(&DataType::Path));

        // But not PATH → TEXT
        assert!(!DataType::Path.can_coerce_to(&DataType::Text));
    }

    #[test]
    fn test_no_implicit_jsonb_coercion() {
        assert!(!DataType::JsonB.can_coerce_to(&DataType::Text));
        assert!(!DataType::Text.can_coerce_to(&DataType::JsonB));
        assert!(!DataType::JsonB.can_coerce_to(&DataType::Int));
    }

    #[test]
    fn test_no_vector_coercion() {
        assert!(!DataType::Vector(128).can_coerce_to(&DataType::Vector(256)));
        assert!(!DataType::Vector(128).can_coerce_to(&DataType::Text));
    }

    #[test]
    fn test_common_type_same() {
        assert_eq!(
            DataType::Int.common_type(&DataType::Int),
            Some(DataType::Int)
        );
    }

    #[test]
    fn test_common_type_numeric_promotion() {
        assert_eq!(
            DataType::Int.common_type(&DataType::BigInt),
            Some(DataType::BigInt)
        );
        assert_eq!(
            DataType::Int.common_type(&DataType::Double),
            Some(DataType::Double)
        );
        assert_eq!(
            DataType::BigInt.common_type(&DataType::Double),
            Some(DataType::Double)
        );
    }

    #[test]
    fn test_common_type_nullable() {
        // INT + INT? = INT?
        assert_eq!(
            DataType::Int.common_type(&DataType::Nullable(Box::new(DataType::Int))),
            Some(DataType::Nullable(Box::new(DataType::Int)))
        );

        // INT + BIGINT? = BIGINT?
        assert_eq!(
            DataType::Int.common_type(&DataType::Nullable(Box::new(DataType::BigInt))),
            Some(DataType::Nullable(Box::new(DataType::BigInt)))
        );
    }

    #[test]
    fn test_common_type_path_text() {
        assert_eq!(
            DataType::Path.common_type(&DataType::Text),
            Some(DataType::Path)
        );
    }

    #[test]
    fn test_common_type_incompatible() {
        assert_eq!(DataType::Int.common_type(&DataType::Text), None);
        assert_eq!(DataType::Path.common_type(&DataType::Int), None);
        assert_eq!(DataType::JsonB.common_type(&DataType::Text), None);
    }

    #[test]
    fn test_is_nullable() {
        assert!(!DataType::Int.is_nullable());
        assert!(DataType::Nullable(Box::new(DataType::Int)).is_nullable());
    }

    #[test]
    fn test_as_nullable() {
        assert_eq!(
            DataType::Int.as_nullable(),
            DataType::Nullable(Box::new(DataType::Int))
        );

        // Already nullable - no double wrapping
        let already_nullable = DataType::Nullable(Box::new(DataType::Int));
        assert_eq!(already_nullable.clone().as_nullable(), already_nullable);
    }

    #[test]
    fn test_base_type() {
        assert_eq!(DataType::Int.base_type(), &DataType::Int);
        assert_eq!(
            DataType::Nullable(Box::new(DataType::Int)).base_type(),
            &DataType::Int
        );
    }

    #[test]
    fn test_intermediate_cast_jsonb_to_all_types() {
        // Direct JSONB → target should NOT be allowed
        assert!(!DataType::JsonB.can_cast_to(&DataType::Boolean));
        assert!(!DataType::JsonB.can_cast_to(&DataType::Int));
        assert!(!DataType::JsonB.can_cast_to(&DataType::BigInt));
        assert!(!DataType::JsonB.can_cast_to(&DataType::Double));
        assert!(!DataType::JsonB.can_cast_to(&DataType::Path));

        // But intermediate via TEXT should work
        assert_eq!(
            DataType::JsonB.get_intermediate_cast_type(&DataType::Boolean),
            Some(DataType::Text)
        );
        assert_eq!(
            DataType::JsonB.get_intermediate_cast_type(&DataType::Int),
            Some(DataType::Text)
        );
        assert_eq!(
            DataType::JsonB.get_intermediate_cast_type(&DataType::BigInt),
            Some(DataType::Text)
        );
        assert_eq!(
            DataType::JsonB.get_intermediate_cast_type(&DataType::Double),
            Some(DataType::Text)
        );
        assert_eq!(
            DataType::JsonB.get_intermediate_cast_type(&DataType::Path),
            Some(DataType::Text)
        );

        // No intermediate needed when direct cast works
        assert_eq!(
            DataType::JsonB.get_intermediate_cast_type(&DataType::Text),
            None
        );

        // No intermediate path for other types
        assert_eq!(
            DataType::Int.get_intermediate_cast_type(&DataType::Path),
            None
        );
    }

    #[test]
    fn test_intermediate_cast_nullable_jsonb() {
        let nullable_jsonb = DataType::Nullable(Box::new(DataType::JsonB));

        // Nullable JSONB → target also needs intermediate
        assert_eq!(
            nullable_jsonb.get_intermediate_cast_type(&DataType::Boolean),
            Some(DataType::Text)
        );
        assert_eq!(
            nullable_jsonb.get_intermediate_cast_type(&DataType::Int),
            Some(DataType::Text)
        );

        // Nullable JSONB → Nullable target
        assert_eq!(
            nullable_jsonb
                .get_intermediate_cast_type(&DataType::Nullable(Box::new(DataType::Boolean))),
            Some(DataType::Text)
        );
    }

    #[test]
    fn test_timestamp_text_explicit_cast() {
        // TEXT → TIMESTAMPTZ should be allowed via explicit cast
        assert!(DataType::Text.can_cast_to(&DataType::TimestampTz));

        // TIMESTAMPTZ → TEXT should be allowed via explicit cast
        assert!(DataType::TimestampTz.can_cast_to(&DataType::Text));

        // But NOT via implicit coercion
        assert!(!DataType::Text.can_coerce_to(&DataType::TimestampTz));
        assert!(!DataType::TimestampTz.can_coerce_to(&DataType::Text));

        // Nullable variants should also work
        assert!(DataType::Nullable(Box::new(DataType::Text)).can_cast_to(&DataType::TimestampTz));
        assert!(DataType::Text.can_cast_to(&DataType::Nullable(Box::new(DataType::TimestampTz))));
    }
}
