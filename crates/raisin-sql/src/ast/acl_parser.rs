//! ACL (Access Control List) statement parser using nom combinators
//!
//! Parses SQL access control statements:
//! - CREATE/ALTER/DROP ROLE
//! - CREATE/ALTER/DROP GROUP
//! - CREATE/ALTER/DROP USER
//! - GRANT / REVOKE
//! - ALTER SECURITY CONFIG / SHOW SECURITY CONFIG
//! - SHOW PERMISSIONS FOR / SHOW EFFECTIVE ROLES FOR
//! - SHOW ROLES / SHOW GROUPS / SHOW USERS
//! - DESCRIBE ROLE / DESCRIBE GROUP / DESCRIBE USER

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::{map, opt, value},
    multi::separated_list1,
    sequence::{delimited, preceded, tuple},
    IResult, Parser,
};

use super::acl::*;

/// Error type for ACL statement parsing
#[derive(Debug, Clone, PartialEq)]
pub struct AclParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for AclParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(pos) = self.position {
            write!(f, "ACL parse error at position {}: {}", pos, self.message)
        } else {
            write!(f, "ACL parse error: {}", self.message)
        }
    }
}

impl std::error::Error for AclParseError {}

// ---------------------------------------------------------------------------
// Guard function
// ---------------------------------------------------------------------------

/// Check if a SQL statement is an ACL statement
pub fn is_acl_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();

    // Two-word prefixes
    if upper.starts_with("CREATE ROLE")
        || upper.starts_with("ALTER ROLE")
        || upper.starts_with("DROP ROLE")
        || upper.starts_with("CREATE GROUP")
        || upper.starts_with("ALTER GROUP")
        || upper.starts_with("DROP GROUP")
        || upper.starts_with("CREATE USER")
        || upper.starts_with("ALTER USER")
        || upper.starts_with("DROP USER")
        || upper.starts_with("DESCRIBE ROLE")
        || upper.starts_with("DESCRIBE GROUP")
        || upper.starts_with("DESCRIBE USER")
    {
        return true;
    }

    // GRANT / REVOKE
    if upper.starts_with("GRANT ") || upper.starts_with("REVOKE ") {
        return true;
    }

    // ALTER SECURITY CONFIG
    if upper.starts_with("ALTER SECURITY CONFIG") {
        return true;
    }

    // SHOW variants - must be selective to avoid matching branch SHOW statements
    if upper.starts_with("SHOW ROLES")
        || upper.starts_with("SHOW GROUPS")
        || upper.starts_with("SHOW USERS")
        || upper.starts_with("SHOW SECURITY CONFIG")
        || upper.starts_with("SHOW PERMISSIONS")
        || upper.starts_with("SHOW EFFECTIVE")
    {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// Public parser entry point
// ---------------------------------------------------------------------------

/// Parse an ACL statement from a SQL string
///
/// Returns `Some(AclStatement)` if the input is a valid ACL statement,
/// `None` if it's not an ACL statement (should be handled by other parsers).
pub fn parse_acl(sql: &str) -> Result<Option<AclStatement>, AclParseError> {
    let trimmed = sql.trim();

    // Strip leading SQL comments
    let statement_start = super::ddl_parser::strip_leading_comments(trimmed);

    if !is_acl_statement(statement_start) {
        return Ok(None);
    }

    let offset_to_statement_start = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    match acl_statement(statement_start) {
        Ok((remaining, stmt)) => {
            let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
            if remaining_trimmed.is_empty() {
                Ok(Some(stmt))
            } else {
                let position_in_statement = statement_start.len() - remaining.len();
                let position = offset_to_statement_start + position_in_statement;
                Err(AclParseError {
                    message: format!("Unexpected trailing content: '{}'", remaining_trimmed),
                    position: Some(position),
                })
            }
        }
        Err(e) => {
            let (position, message) = match &e {
                nom::Err::Failure(err) | nom::Err::Error(err) => {
                    let pos_in_statement = statement_start.len() - err.input.len();
                    let remaining = err.input.trim();
                    let problematic: String = remaining
                        .chars()
                        .take(30)
                        .take_while(|c| *c != '\n')
                        .collect();
                    (
                        Some(offset_to_statement_start + pos_in_statement),
                        format!("Parse error near: '{}'", problematic.trim()),
                    )
                }
                nom::Err::Incomplete(_) => (None, "Incomplete ACL statement".to_string()),
            };
            Err(AclParseError { message, position })
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level statement dispatcher
// ---------------------------------------------------------------------------

fn acl_statement(input: &str) -> IResult<&str, AclStatement> {
    alt((
        // Role statements
        map(create_role, AclStatement::CreateRole),
        map(alter_role, AclStatement::AlterRole),
        map(drop_role, AclStatement::DropRole),
        map(show_roles, AclStatement::ShowRoles),
        map(describe_role, AclStatement::DescribeRole),
        // Group statements
        map(create_group, AclStatement::CreateGroup),
        map(alter_group, AclStatement::AlterGroup),
        map(drop_group, AclStatement::DropGroup),
        map(show_groups, AclStatement::ShowGroups),
        map(describe_group, AclStatement::DescribeGroup),
        // User statements
        map(create_user, AclStatement::CreateUser),
        map(alter_user, AclStatement::AlterUser),
        map(drop_user, AclStatement::DropUser),
        map(show_users, AclStatement::ShowUsers),
        map(describe_user, AclStatement::DescribeUser),
        // Grant / Revoke
        map(grant_stmt, AclStatement::Grant),
        map(revoke_stmt, AclStatement::Revoke),
        // Security config
        map(alter_security_config, AclStatement::AlterSecurityConfig),
        map(show_security_config, AclStatement::ShowSecurityConfig),
        // Introspection
        map(show_permissions_for, AclStatement::ShowPermissionsFor),
        map(
            show_effective_roles_for,
            AclStatement::ShowEffectiveRolesFor,
        ),
    ))
    .parse(input)
}

// ---------------------------------------------------------------------------
// Primitive parsers
// ---------------------------------------------------------------------------

/// Parse a single-quoted string: 'content'
fn quoted_string(input: &str) -> IResult<&str, &str> {
    alt((
        delimited(char('\''), take_while1(|c| c != '\''), char('\'')),
        delimited(char('"'), take_while1(|c| c != '"'), char('"')),
    ))
    .parse(input)
}

/// Parse an unquoted identifier: alphanumeric + underscore + hyphen + dot
fn identifier(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_').parse(input)
}

/// Parse a parenthesized, comma-separated list of quoted strings: ('a', 'b', 'c')
fn string_list(input: &str) -> IResult<&str, Vec<String>> {
    delimited(
        (char('('), multispace0),
        separated_list1(
            (multispace0, char(','), multispace0),
            map(quoted_string, |s: &str| s.to_string()),
        ),
        (multispace0, char(')')),
    )
    .parse(input)
}

/// Parse a parenthesized, comma-separated list of identifiers: (a, b, c)
fn identifier_list(input: &str) -> IResult<&str, Vec<String>> {
    delimited(
        (char('('), multispace0),
        separated_list1(
            (multispace0, char(','), multispace0),
            map(identifier, |s: &str| s.to_string()),
        ),
        (multispace0, char(')')),
    )
    .parse(input)
}

/// Parse a single operation keyword
fn operation(input: &str) -> IResult<&str, Operation> {
    alt((
        value(Operation::Create, tag_no_case("CREATE")),
        value(Operation::Read, tag_no_case("READ")),
        value(Operation::Update, tag_no_case("UPDATE")),
        value(Operation::Delete, tag_no_case("DELETE")),
        value(Operation::Translate, tag_no_case("TRANSLATE")),
        value(Operation::Relate, tag_no_case("RELATE")),
        value(Operation::Unrelate, tag_no_case("UNRELATE")),
    ))
    .parse(input)
}

/// Parse a comma-separated list of operations
fn operation_list(input: &str) -> IResult<&str, Vec<Operation>> {
    separated_list1((multispace0, char(','), multispace0), operation).parse(input)
}

/// Parse a boolean value: true or false
fn boolean_value(input: &str) -> IResult<&str, bool> {
    alt((
        value(true, tag_no_case("true")),
        value(false, tag_no_case("false")),
    ))
    .parse(input)
}

// ---------------------------------------------------------------------------
// Permission grant parser
// ---------------------------------------------------------------------------

/// Parse a single ALLOW permission grant clause
fn permission_grant(input: &str) -> IResult<&str, PermissionGrant> {
    let (input, _) = tag_no_case("ALLOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, operations) = operation_list(input)?;

    // Optional: ON 'workspace'
    let (input, workspace) = opt(preceded(
        tuple((multispace1, tag_no_case("ON"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    // Required: PATH 'pattern'
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("PATH").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, path) = quoted_string(input)?;

    // Optional: BRANCH 'pattern'
    let (input, branch_pattern) = opt(preceded(
        tuple((multispace1, tag_no_case("BRANCH"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    // Optional: NODE TYPES ('type_a', 'type_b')
    let (input, node_types) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("NODE"),
            multispace1,
            tag_no_case("TYPES"),
            multispace0,
        )),
        string_list,
    ))
    .parse(input)?;

    // Optional: FIELDS (field_a, field_b)
    let (input, fields) = opt(preceded(
        tuple((multispace1, tag_no_case("FIELDS"), multispace0)),
        identifier_list,
    ))
    .parse(input)?;

    // Optional: EXCEPT FIELDS (field_x)
    let (input, except_fields) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("EXCEPT"),
            multispace1,
            tag_no_case("FIELDS"),
            multispace0,
        )),
        identifier_list,
    ))
    .parse(input)?;

    // Optional: WHERE <raw rel expression>
    // Capture everything after WHERE until we hit a comma or closing paren at the
    // permission-list level.
    let (input, condition) = opt(preceded(
        tuple((multispace1, tag_no_case("WHERE"), multispace1)),
        where_condition,
    ))
    .parse(input)?;

    Ok((
        input,
        PermissionGrant {
            operations,
            workspace: workspace.map(|s| s.to_string()),
            path: path.to_string(),
            branch_pattern: branch_pattern.map(|s| s.to_string()),
            node_types,
            fields,
            except_fields,
            condition: condition.map(|s| s.to_string()),
        },
    ))
}

/// Parse a WHERE condition: capture raw text until we reach a boundary
/// (comma at paren depth 0, or closing paren that would go negative).
fn where_condition(input: &str) -> IResult<&str, &str> {
    let mut depth: i32 = 0;
    let mut end = 0;
    let bytes = input.as_bytes();

    while end < bytes.len() {
        match bytes[end] {
            b'(' => {
                depth += 1;
                end += 1;
            }
            b')' => {
                if depth <= 0 {
                    // This closing paren belongs to the outer permission_list
                    break;
                }
                depth -= 1;
                end += 1;
            }
            b',' if depth == 0 => {
                // Comma at top level = separator between permission grants
                break;
            }
            b'\'' => {
                // Skip quoted string content
                end += 1;
                while end < bytes.len() && bytes[end] != b'\'' {
                    end += 1;
                }
                if end < bytes.len() {
                    end += 1; // skip closing quote
                }
            }
            _ => {
                end += 1;
            }
        }
    }

    let captured = input[..end].trim();
    if captured.is_empty() {
        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TakeWhile1,
        )))
    } else {
        Ok((&input[end..], captured))
    }
}

/// Parse a parenthesized list of permission grants
fn permission_list(input: &str) -> IResult<&str, Vec<PermissionGrant>> {
    delimited(
        (char('('), multispace0),
        separated_list1((multispace0, char(','), multispace0), permission_grant),
        (multispace0, char(')')),
    )
    .parse(input)
}

// ---------------------------------------------------------------------------
// Role statements
// ---------------------------------------------------------------------------

fn create_role(input: &str) -> IResult<&str, CreateRole> {
    let (input, _) = tuple((
        tag_no_case("CREATE"),
        multispace1,
        tag_no_case("ROLE"),
        multispace1,
    ))
    .parse(input)?;
    let (input, role_id) = quoted_string(input)?;

    // Optional DESCRIPTION
    let (input, description) = opt(preceded(
        tuple((multispace1, tag_no_case("DESCRIPTION"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    // Optional INHERITS ('a', 'b')
    let (input, inherits) = opt(preceded(
        tuple((multispace1, tag_no_case("INHERITS"), multispace0)),
        string_list,
    ))
    .parse(input)?;

    // Optional PERMISSIONS (...)
    let (input, permissions) = opt(preceded(
        tuple((multispace1, tag_no_case("PERMISSIONS"), multispace0)),
        permission_list,
    ))
    .parse(input)?;

    Ok((
        input,
        CreateRole {
            role_id: role_id.to_string(),
            description: description.map(|s| s.to_string()),
            inherits: inherits.unwrap_or_default(),
            permissions: permissions.unwrap_or_default(),
        },
    ))
}

fn alter_role(input: &str) -> IResult<&str, AlterRole> {
    let (input, _) = tuple((
        tag_no_case("ALTER"),
        multispace1,
        tag_no_case("ROLE"),
        multispace1,
    ))
    .parse(input)?;
    let (input, role_id) = quoted_string(input)?;
    let (input, _) = multispace1.parse(input)?;

    let (input, action) = alt((
        alter_role_add_permission,
        alter_role_drop_permission,
        alter_role_add_inherits,
        alter_role_drop_inherits,
        alter_role_set_description,
    ))
    .parse(input)?;

    Ok((
        input,
        AlterRole {
            role_id: role_id.to_string(),
            action,
        },
    ))
}

fn alter_role_add_permission(input: &str) -> IResult<&str, AlterRoleAction> {
    let (input, _) = tuple((
        tag_no_case("ADD"),
        multispace1,
        tag_no_case("PERMISSION"),
        multispace1,
    ))
    .parse(input)?;
    let (input, grant) = permission_grant(input)?;
    Ok((input, AlterRoleAction::AddPermission(grant)))
}

fn alter_role_drop_permission(input: &str) -> IResult<&str, AlterRoleAction> {
    let (input, _) = tuple((
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("PERMISSION"),
        multispace1,
    ))
    .parse(input)?;
    let (input, idx) = digit1(input)?;
    let idx: usize = idx.parse().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit))
    })?;
    Ok((input, AlterRoleAction::DropPermission(idx)))
}

fn alter_role_add_inherits(input: &str) -> IResult<&str, AlterRoleAction> {
    let (input, _) = tuple((
        tag_no_case("ADD"),
        multispace1,
        tag_no_case("INHERITS"),
        multispace0,
    ))
    .parse(input)?;
    let (input, roles) = string_list(input)?;
    Ok((input, AlterRoleAction::AddInherits(roles)))
}

fn alter_role_drop_inherits(input: &str) -> IResult<&str, AlterRoleAction> {
    let (input, _) = tuple((
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("INHERITS"),
        multispace0,
    ))
    .parse(input)?;
    let (input, roles) = string_list(input)?;
    Ok((input, AlterRoleAction::DropInherits(roles)))
}

fn alter_role_set_description(input: &str) -> IResult<&str, AlterRoleAction> {
    let (input, _) = tuple((
        tag_no_case("SET"),
        multispace1,
        tag_no_case("DESCRIPTION"),
        multispace1,
    ))
    .parse(input)?;
    let (input, desc) = quoted_string(input)?;
    Ok((input, AlterRoleAction::SetDescription(desc.to_string())))
}

fn drop_role(input: &str) -> IResult<&str, DropRole> {
    let (input, _) = tuple((
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("ROLE"),
        multispace1,
    ))
    .parse(input)?;

    let (input, if_exists) = opt(tuple((
        tag_no_case("IF"),
        multispace1,
        tag_no_case("EXISTS"),
        multispace1,
    )))
    .parse(input)?;

    let (input, role_id) = quoted_string(input)?;

    Ok((
        input,
        DropRole {
            role_id: role_id.to_string(),
            if_exists: if_exists.is_some(),
        },
    ))
}

fn show_roles(input: &str) -> IResult<&str, ShowRoles> {
    let (input, _) =
        tuple((tag_no_case("SHOW"), multispace1, tag_no_case("ROLES"))).parse(input)?;

    let (input, like_pattern) = opt(preceded(
        tuple((multispace1, tag_no_case("LIKE"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    Ok((
        input,
        ShowRoles {
            like_pattern: like_pattern.map(|s| s.to_string()),
        },
    ))
}

fn describe_role(input: &str) -> IResult<&str, DescribeRole> {
    let (input, _) = tuple((
        tag_no_case("DESCRIBE"),
        multispace1,
        tag_no_case("ROLE"),
        multispace1,
    ))
    .parse(input)?;
    let (input, role_id) = quoted_string(input)?;
    Ok((
        input,
        DescribeRole {
            role_id: role_id.to_string(),
        },
    ))
}

// ---------------------------------------------------------------------------
// Group statements
// ---------------------------------------------------------------------------

fn create_group(input: &str) -> IResult<&str, CreateGroup> {
    let (input, _) = tuple((
        tag_no_case("CREATE"),
        multispace1,
        tag_no_case("GROUP"),
        multispace1,
    ))
    .parse(input)?;
    let (input, group_id) = quoted_string(input)?;

    // Optional DESCRIPTION
    let (input, description) = opt(preceded(
        tuple((multispace1, tag_no_case("DESCRIPTION"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    // Optional ROLES ('a', 'b')
    let (input, roles) = opt(preceded(
        tuple((multispace1, tag_no_case("ROLES"), multispace0)),
        string_list,
    ))
    .parse(input)?;

    Ok((
        input,
        CreateGroup {
            group_id: group_id.to_string(),
            description: description.map(|s| s.to_string()),
            roles: roles.unwrap_or_default(),
        },
    ))
}

fn alter_group(input: &str) -> IResult<&str, AlterGroup> {
    let (input, _) = tuple((
        tag_no_case("ALTER"),
        multispace1,
        tag_no_case("GROUP"),
        multispace1,
    ))
    .parse(input)?;
    let (input, group_id) = quoted_string(input)?;
    let (input, _) = multispace1.parse(input)?;

    let (input, action) = alt((
        alter_group_add_roles,
        alter_group_drop_roles,
        alter_group_set_description,
    ))
    .parse(input)?;

    Ok((
        input,
        AlterGroup {
            group_id: group_id.to_string(),
            action,
        },
    ))
}

fn alter_group_add_roles(input: &str) -> IResult<&str, AlterGroupAction> {
    let (input, _) = tuple((
        tag_no_case("ADD"),
        multispace1,
        tag_no_case("ROLES"),
        multispace0,
    ))
    .parse(input)?;
    let (input, roles) = string_list(input)?;
    Ok((input, AlterGroupAction::AddRoles(roles)))
}

fn alter_group_drop_roles(input: &str) -> IResult<&str, AlterGroupAction> {
    let (input, _) = tuple((
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("ROLES"),
        multispace0,
    ))
    .parse(input)?;
    let (input, roles) = string_list(input)?;
    Ok((input, AlterGroupAction::DropRoles(roles)))
}

fn alter_group_set_description(input: &str) -> IResult<&str, AlterGroupAction> {
    let (input, _) = tuple((
        tag_no_case("SET"),
        multispace1,
        tag_no_case("DESCRIPTION"),
        multispace1,
    ))
    .parse(input)?;
    let (input, desc) = quoted_string(input)?;
    Ok((input, AlterGroupAction::SetDescription(desc.to_string())))
}

fn drop_group(input: &str) -> IResult<&str, DropGroup> {
    let (input, _) = tuple((
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("GROUP"),
        multispace1,
    ))
    .parse(input)?;

    let (input, if_exists) = opt(tuple((
        tag_no_case("IF"),
        multispace1,
        tag_no_case("EXISTS"),
        multispace1,
    )))
    .parse(input)?;

    let (input, group_id) = quoted_string(input)?;

    Ok((
        input,
        DropGroup {
            group_id: group_id.to_string(),
            if_exists: if_exists.is_some(),
        },
    ))
}

fn show_groups(input: &str) -> IResult<&str, ShowGroups> {
    let (input, _) =
        tuple((tag_no_case("SHOW"), multispace1, tag_no_case("GROUPS"))).parse(input)?;

    let (input, like_pattern) = opt(preceded(
        tuple((multispace1, tag_no_case("LIKE"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    Ok((
        input,
        ShowGroups {
            like_pattern: like_pattern.map(|s| s.to_string()),
        },
    ))
}

fn describe_group(input: &str) -> IResult<&str, DescribeGroup> {
    let (input, _) = tuple((
        tag_no_case("DESCRIBE"),
        multispace1,
        tag_no_case("GROUP"),
        multispace1,
    ))
    .parse(input)?;
    let (input, group_id) = quoted_string(input)?;
    Ok((
        input,
        DescribeGroup {
            group_id: group_id.to_string(),
        },
    ))
}

// ---------------------------------------------------------------------------
// User statements
// ---------------------------------------------------------------------------

fn create_user(input: &str) -> IResult<&str, CreateUser> {
    let (input, _) = tuple((
        tag_no_case("CREATE"),
        multispace1,
        tag_no_case("USER"),
        multispace1,
    ))
    .parse(input)?;
    let (input, user_id) = quoted_string(input)?;

    // Required: EMAIL
    let (input, _) = tuple((multispace1, tag_no_case("EMAIL"), multispace1)).parse(input)?;
    let (input, email) = quoted_string(input)?;

    // Optional: DISPLAY NAME
    let (input, display_name) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("DISPLAY"),
            multispace1,
            tag_no_case("NAME"),
            multispace1,
        )),
        quoted_string,
    ))
    .parse(input)?;

    // Optional: ROLES ('a', 'b')
    let (input, roles) = opt(preceded(
        tuple((multispace1, tag_no_case("ROLES"), multispace0)),
        string_list,
    ))
    .parse(input)?;

    // Optional: GROUPS ('a', 'b')
    let (input, groups) = opt(preceded(
        tuple((multispace1, tag_no_case("GROUPS"), multispace0)),
        string_list,
    ))
    .parse(input)?;

    // Optional: CAN LOGIN true/false
    let (input, can_login) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("CAN"),
            multispace1,
            tag_no_case("LOGIN"),
            multispace1,
        )),
        boolean_value,
    ))
    .parse(input)?;

    // Optional: BIRTH DATE 'date'
    let (input, birth_date) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("BIRTH"),
            multispace1,
            tag_no_case("DATE"),
            multispace1,
        )),
        quoted_string,
    ))
    .parse(input)?;

    // Optional: IN FOLDER 'path'
    let (input, folder) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("IN"),
            multispace1,
            tag_no_case("FOLDER"),
            multispace1,
        )),
        quoted_string,
    ))
    .parse(input)?;

    Ok((
        input,
        CreateUser {
            user_id: user_id.to_string(),
            email: email.to_string(),
            display_name: display_name.map(|s| s.to_string()),
            roles: roles.unwrap_or_default(),
            groups: groups.unwrap_or_default(),
            can_login,
            birth_date: birth_date.map(|s| s.to_string()),
            folder: folder.map(|s| s.to_string()),
        },
    ))
}

fn alter_user(input: &str) -> IResult<&str, AlterUser> {
    let (input, _) = tuple((
        tag_no_case("ALTER"),
        multispace1,
        tag_no_case("USER"),
        multispace1,
    ))
    .parse(input)?;
    let (input, user_id) = quoted_string(input)?;
    let (input, _) = multispace1.parse(input)?;

    let (input, action) = alt((
        alter_user_add_roles,
        alter_user_drop_roles,
        alter_user_add_groups,
        alter_user_drop_groups,
        alter_user_set_email,
        alter_user_set_display_name,
        alter_user_set_can_login,
        alter_user_set_birth_date,
    ))
    .parse(input)?;

    Ok((
        input,
        AlterUser {
            user_id: user_id.to_string(),
            action,
        },
    ))
}

fn alter_user_add_roles(input: &str) -> IResult<&str, AlterUserAction> {
    let (input, _) = tuple((
        tag_no_case("ADD"),
        multispace1,
        tag_no_case("ROLES"),
        multispace0,
    ))
    .parse(input)?;
    let (input, roles) = string_list(input)?;
    Ok((input, AlterUserAction::AddRoles(roles)))
}

fn alter_user_drop_roles(input: &str) -> IResult<&str, AlterUserAction> {
    let (input, _) = tuple((
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("ROLES"),
        multispace0,
    ))
    .parse(input)?;
    let (input, roles) = string_list(input)?;
    Ok((input, AlterUserAction::DropRoles(roles)))
}

fn alter_user_add_groups(input: &str) -> IResult<&str, AlterUserAction> {
    let (input, _) = tuple((
        tag_no_case("ADD"),
        multispace1,
        tag_no_case("GROUPS"),
        multispace0,
    ))
    .parse(input)?;
    let (input, groups) = string_list(input)?;
    Ok((input, AlterUserAction::AddGroups(groups)))
}

fn alter_user_drop_groups(input: &str) -> IResult<&str, AlterUserAction> {
    let (input, _) = tuple((
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("GROUPS"),
        multispace0,
    ))
    .parse(input)?;
    let (input, groups) = string_list(input)?;
    Ok((input, AlterUserAction::DropGroups(groups)))
}

fn alter_user_set_email(input: &str) -> IResult<&str, AlterUserAction> {
    let (input, _) = tuple((
        tag_no_case("SET"),
        multispace1,
        tag_no_case("EMAIL"),
        multispace1,
    ))
    .parse(input)?;
    let (input, email) = quoted_string(input)?;
    Ok((input, AlterUserAction::SetEmail(email.to_string())))
}

fn alter_user_set_display_name(input: &str) -> IResult<&str, AlterUserAction> {
    let (input, _) = tuple((
        tag_no_case("SET"),
        multispace1,
        tag_no_case("DISPLAY"),
        multispace1,
        tag_no_case("NAME"),
        multispace1,
    ))
    .parse(input)?;
    let (input, name) = quoted_string(input)?;
    Ok((input, AlterUserAction::SetDisplayName(name.to_string())))
}

fn alter_user_set_can_login(input: &str) -> IResult<&str, AlterUserAction> {
    let (input, _) = tuple((
        tag_no_case("SET"),
        multispace1,
        tag_no_case("CAN"),
        multispace1,
        tag_no_case("LOGIN"),
        multispace1,
    ))
    .parse(input)?;
    let (input, val) = boolean_value(input)?;
    Ok((input, AlterUserAction::SetCanLogin(val)))
}

fn alter_user_set_birth_date(input: &str) -> IResult<&str, AlterUserAction> {
    let (input, _) = tuple((
        tag_no_case("SET"),
        multispace1,
        tag_no_case("BIRTH"),
        multispace1,
        tag_no_case("DATE"),
        multispace1,
    ))
    .parse(input)?;
    let (input, date) = quoted_string(input)?;
    Ok((input, AlterUserAction::SetBirthDate(date.to_string())))
}

fn drop_user(input: &str) -> IResult<&str, DropUser> {
    let (input, _) = tuple((
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("USER"),
        multispace1,
    ))
    .parse(input)?;

    let (input, if_exists) = opt(tuple((
        tag_no_case("IF"),
        multispace1,
        tag_no_case("EXISTS"),
        multispace1,
    )))
    .parse(input)?;

    let (input, user_id) = quoted_string(input)?;

    Ok((
        input,
        DropUser {
            user_id: user_id.to_string(),
            if_exists: if_exists.is_some(),
        },
    ))
}

fn show_users(input: &str) -> IResult<&str, ShowUsers> {
    let (input, _) =
        tuple((tag_no_case("SHOW"), multispace1, tag_no_case("USERS"))).parse(input)?;

    let (input, like_pattern) = opt(preceded(
        tuple((multispace1, tag_no_case("LIKE"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    let (input, in_group) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("IN"),
            multispace1,
            tag_no_case("GROUP"),
            multispace1,
        )),
        quoted_string,
    ))
    .parse(input)?;

    let (input, with_role) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("WITH"),
            multispace1,
            tag_no_case("ROLE"),
            multispace1,
        )),
        quoted_string,
    ))
    .parse(input)?;

    Ok((
        input,
        ShowUsers {
            like_pattern: like_pattern.map(|s| s.to_string()),
            in_group: in_group.map(|s| s.to_string()),
            with_role: with_role.map(|s| s.to_string()),
        },
    ))
}

fn describe_user(input: &str) -> IResult<&str, DescribeUser> {
    let (input, _) = tuple((
        tag_no_case("DESCRIBE"),
        multispace1,
        tag_no_case("USER"),
        multispace1,
    ))
    .parse(input)?;
    let (input, user_id) = quoted_string(input)?;
    Ok((
        input,
        DescribeUser {
            user_id: user_id.to_string(),
        },
    ))
}

// ---------------------------------------------------------------------------
// GRANT / REVOKE statements
// ---------------------------------------------------------------------------

fn grant_stmt(input: &str) -> IResult<&str, Grant> {
    let (input, _) = tuple((tag_no_case("GRANT"), multispace1)).parse(input)?;

    // Parse grant items: ROLE 'x' | ROLES ('x', 'y') | GROUP 'x' | GROUPS ('x', 'y')
    let (input, grants) = grant_items(input)?;

    // TO USER 'x' | TO GROUP 'x'
    let (input, _) = tuple((multispace1, tag_no_case("TO"), multispace1)).parse(input)?;
    let (input, target) = grant_target(input)?;

    Ok((input, Grant { target, grants }))
}

fn grant_items(input: &str) -> IResult<&str, Vec<GrantItem>> {
    alt((
        // ROLES ('x', 'y')
        map(
            preceded(tuple((tag_no_case("ROLES"), multispace0)), string_list),
            |roles| roles.into_iter().map(GrantItem::Role).collect(),
        ),
        // ROLE 'x'
        map(
            preceded(tuple((tag_no_case("ROLE"), multispace1)), quoted_string),
            |s| vec![GrantItem::Role(s.to_string())],
        ),
        // GROUPS ('x', 'y')
        map(
            preceded(tuple((tag_no_case("GROUPS"), multispace0)), string_list),
            |groups| groups.into_iter().map(GrantItem::Group).collect(),
        ),
        // GROUP 'x'
        map(
            preceded(tuple((tag_no_case("GROUP"), multispace1)), quoted_string),
            |s| vec![GrantItem::Group(s.to_string())],
        ),
    ))
    .parse(input)
}

fn grant_target(input: &str) -> IResult<&str, GrantTarget> {
    alt((
        map(
            preceded(tuple((tag_no_case("USER"), multispace1)), quoted_string),
            |s| GrantTarget::User(s.to_string()),
        ),
        map(
            preceded(tuple((tag_no_case("GROUP"), multispace1)), quoted_string),
            |s| GrantTarget::Group(s.to_string()),
        ),
    ))
    .parse(input)
}

fn revoke_stmt(input: &str) -> IResult<&str, Revoke> {
    let (input, _) = tuple((tag_no_case("REVOKE"), multispace1)).parse(input)?;

    // Parse revoke items: same structure as grant items
    let (input, revocations) = revoke_items(input)?;

    // FROM USER 'x' | FROM GROUP 'x'
    let (input, _) = tuple((multispace1, tag_no_case("FROM"), multispace1)).parse(input)?;
    let (input, target) = revoke_target(input)?;

    Ok((
        input,
        Revoke {
            target,
            revocations,
        },
    ))
}

fn revoke_items(input: &str) -> IResult<&str, Vec<RevokeItem>> {
    alt((
        map(
            preceded(tuple((tag_no_case("ROLES"), multispace0)), string_list),
            |roles| roles.into_iter().map(RevokeItem::Role).collect(),
        ),
        map(
            preceded(tuple((tag_no_case("ROLE"), multispace1)), quoted_string),
            |s| vec![RevokeItem::Role(s.to_string())],
        ),
        map(
            preceded(tuple((tag_no_case("GROUPS"), multispace0)), string_list),
            |groups| groups.into_iter().map(RevokeItem::Group).collect(),
        ),
        map(
            preceded(tuple((tag_no_case("GROUP"), multispace1)), quoted_string),
            |s| vec![RevokeItem::Group(s.to_string())],
        ),
    ))
    .parse(input)
}

fn revoke_target(input: &str) -> IResult<&str, RevokeTarget> {
    alt((
        map(
            preceded(tuple((tag_no_case("USER"), multispace1)), quoted_string),
            |s| RevokeTarget::User(s.to_string()),
        ),
        map(
            preceded(tuple((tag_no_case("GROUP"), multispace1)), quoted_string),
            |s| RevokeTarget::Group(s.to_string()),
        ),
    ))
    .parse(input)
}

// ---------------------------------------------------------------------------
// Security config statements
// ---------------------------------------------------------------------------

fn alter_security_config(input: &str) -> IResult<&str, AlterSecurityConfig> {
    let (input, _) = tuple((
        tag_no_case("ALTER"),
        multispace1,
        tag_no_case("SECURITY"),
        multispace1,
        tag_no_case("CONFIG"),
        multispace1,
    ))
    .parse(input)?;
    let (input, workspace_pattern) = quoted_string(input)?;

    // Parse one or more SET clauses
    let (input, settings) =
        nom::multi::many1(preceded(multispace1, security_config_setting)).parse(input)?;

    Ok((
        input,
        AlterSecurityConfig {
            workspace_pattern: workspace_pattern.to_string(),
            settings,
        },
    ))
}

fn security_config_setting(input: &str) -> IResult<&str, SecurityConfigSetting> {
    let (input, _) = tuple((tag_no_case("SET"), multispace1)).parse(input)?;

    alt((
        security_config_default_policy,
        security_config_anonymous_enabled,
        security_config_anonymous_role,
        security_config_interface_setting,
    ))
    .parse(input)
}

fn security_config_default_policy(input: &str) -> IResult<&str, SecurityConfigSetting> {
    let (input, _) = tuple((
        tag_no_case("DEFAULT"),
        multispace1,
        tag_no_case("POLICY"),
        multispace1,
    ))
    .parse(input)?;
    let (input, policy) = quoted_string(input)?;
    Ok((
        input,
        SecurityConfigSetting::DefaultPolicy(policy.to_string()),
    ))
}

fn security_config_anonymous_enabled(input: &str) -> IResult<&str, SecurityConfigSetting> {
    let (input, _) = tuple((
        tag_no_case("ANONYMOUS"),
        multispace1,
        tag_no_case("ENABLED"),
        multispace1,
    ))
    .parse(input)?;
    let (input, enabled) = boolean_value(input)?;
    Ok((input, SecurityConfigSetting::AnonymousEnabled(enabled)))
}

fn security_config_anonymous_role(input: &str) -> IResult<&str, SecurityConfigSetting> {
    let (input, _) = tuple((
        tag_no_case("ANONYMOUS"),
        multispace1,
        tag_no_case("ROLE"),
        multispace1,
    ))
    .parse(input)?;
    let (input, role) = quoted_string(input)?;
    Ok((
        input,
        SecurityConfigSetting::AnonymousRole(role.to_string()),
    ))
}

fn security_config_interface_setting(input: &str) -> IResult<&str, SecurityConfigSetting> {
    let (input, _) = tuple((tag_no_case("INTERFACE"), multispace1)).parse(input)?;
    let (input, interface) = identifier(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse the key (e.g., ANONYMOUS ENABLED) - capture as identifier words
    // We support patterns like: ANONYMOUS ENABLED true
    let (input, key_and_value) = alt((
        // ANONYMOUS ENABLED <bool>
        map(
            tuple((
                tag_no_case("ANONYMOUS"),
                multispace1,
                tag_no_case("ENABLED"),
                multispace1,
                alt((tag_no_case("true"), tag_no_case("false"))),
            )),
            |(_, _, _, _, val): (&str, &str, &str, &str, &str)| {
                ("anonymous_enabled".to_string(), val.to_lowercase())
            },
        ),
        // Generic: KEY VALUE (identifier + quoted_string)
        map(
            tuple((identifier, multispace1, quoted_string)),
            |(key, _, val): (&str, &str, &str)| (key.to_lowercase(), val.to_string()),
        ),
    ))
    .parse(input)?;

    Ok((
        input,
        SecurityConfigSetting::InterfaceSetting {
            interface: interface.to_string(),
            key: key_and_value.0,
            value: key_and_value.1,
        },
    ))
}

fn show_security_config(input: &str) -> IResult<&str, ShowSecurityConfig> {
    let (input, _) = tuple((
        tag_no_case("SHOW"),
        multispace1,
        tag_no_case("SECURITY"),
        multispace1,
        tag_no_case("CONFIG"),
    ))
    .parse(input)?;

    let (input, workspace) = opt(preceded(
        tuple((multispace1, tag_no_case("FOR"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    Ok((
        input,
        ShowSecurityConfig {
            workspace: workspace.map(|s| s.to_string()),
        },
    ))
}

// ---------------------------------------------------------------------------
// Introspection statements
// ---------------------------------------------------------------------------

fn show_permissions_for(input: &str) -> IResult<&str, ShowPermissionsFor> {
    let (input, _) = tuple((
        tag_no_case("SHOW"),
        multispace1,
        tag_no_case("PERMISSIONS"),
        multispace1,
        tag_no_case("FOR"),
        multispace1,
        tag_no_case("USER"),
        multispace1,
    ))
    .parse(input)?;
    let (input, user_id) = quoted_string(input)?;

    let (input, workspace) = opt(preceded(
        tuple((multispace1, tag_no_case("ON"), multispace1)),
        quoted_string,
    ))
    .parse(input)?;

    Ok((
        input,
        ShowPermissionsFor {
            user_id: user_id.to_string(),
            workspace: workspace.map(|s| s.to_string()),
        },
    ))
}

fn show_effective_roles_for(input: &str) -> IResult<&str, ShowEffectiveRolesFor> {
    let (input, _) = tuple((
        tag_no_case("SHOW"),
        multispace1,
        tag_no_case("EFFECTIVE"),
        multispace1,
        tag_no_case("ROLES"),
        multispace1,
        tag_no_case("FOR"),
        multispace1,
        tag_no_case("USER"),
        multispace1,
    ))
    .parse(input)?;
    let (input, user_id) = quoted_string(input)?;

    Ok((
        input,
        ShowEffectiveRolesFor {
            user_id: user_id.to_string(),
        },
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Role tests
    // =========================================================================

    #[test]
    fn test_create_role_minimal() {
        let result = parse_acl("CREATE ROLE 'viewer'").unwrap().unwrap();
        match result {
            AclStatement::CreateRole(cr) => {
                assert_eq!(cr.role_id, "viewer");
                assert!(cr.description.is_none());
                assert!(cr.inherits.is_empty());
                assert!(cr.permissions.is_empty());
            }
            other => panic!("Expected CreateRole, got {:?}", other),
        }
    }

    #[test]
    fn test_create_role_full() {
        let sql = r#"CREATE ROLE 'content-editor'
            DESCRIPTION 'Can edit content'
            INHERITS ('viewer', 'commenter')
            PERMISSIONS (
                ALLOW READ, UPDATE ON 'site-*' PATH '/content/**'
                    BRANCH 'main'
                    NODE TYPES ('article', 'page')
                    FIELDS (title, body)
                    EXCEPT FIELDS (internal_notes)
                    WHERE status = 'published'
            )"#;
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::CreateRole(cr) => {
                assert_eq!(cr.role_id, "content-editor");
                assert_eq!(cr.description.as_deref(), Some("Can edit content"));
                assert_eq!(cr.inherits, vec!["viewer", "commenter"]);
                assert_eq!(cr.permissions.len(), 1);
                let p = &cr.permissions[0];
                assert_eq!(p.operations, vec![Operation::Read, Operation::Update]);
                assert_eq!(p.workspace.as_deref(), Some("site-*"));
                assert_eq!(p.path, "/content/**");
                assert_eq!(p.branch_pattern.as_deref(), Some("main"));
                assert_eq!(
                    p.node_types.as_ref().unwrap(),
                    &vec!["article".to_string(), "page".to_string()]
                );
                assert_eq!(
                    p.fields.as_ref().unwrap(),
                    &vec!["title".to_string(), "body".to_string()]
                );
                assert_eq!(
                    p.except_fields.as_ref().unwrap(),
                    &vec!["internal_notes".to_string()]
                );
                assert_eq!(p.condition.as_deref(), Some("status = 'published'"));
            }
            other => panic!("Expected CreateRole, got {:?}", other),
        }
    }

    #[test]
    fn test_create_role_permission_with_condition() {
        let sql = r#"CREATE ROLE 'restricted'
            PERMISSIONS (
                ALLOW READ PATH '/docs/**' WHERE owner = 'current_user'
            )"#;
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::CreateRole(cr) => {
                assert_eq!(cr.permissions.len(), 1);
                let p = &cr.permissions[0];
                assert_eq!(p.operations, vec![Operation::Read]);
                assert_eq!(p.path, "/docs/**");
                assert_eq!(p.condition.as_deref(), Some("owner = 'current_user'"));
                assert!(p.workspace.is_none());
                assert!(p.fields.is_none());
                assert!(p.except_fields.is_none());
            }
            other => panic!("Expected CreateRole, got {:?}", other),
        }
    }

    #[test]
    fn test_create_role_permission_with_fields() {
        let sql = r#"CREATE ROLE 'field-reader'
            PERMISSIONS (
                ALLOW READ PATH '/content/**' FIELDS (title, summary) EXCEPT FIELDS (secret_key)
            )"#;
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::CreateRole(cr) => {
                assert_eq!(cr.permissions.len(), 1);
                let p = &cr.permissions[0];
                assert_eq!(
                    p.fields.as_ref().unwrap(),
                    &vec!["title".to_string(), "summary".to_string()]
                );
                assert_eq!(
                    p.except_fields.as_ref().unwrap(),
                    &vec!["secret_key".to_string()]
                );
            }
            other => panic!("Expected CreateRole, got {:?}", other),
        }
    }

    #[test]
    fn test_create_role_multiple_permissions() {
        let sql = r#"CREATE ROLE 'admin'
            PERMISSIONS (
                ALLOW CREATE, READ, UPDATE, DELETE PATH '/**',
                ALLOW RELATE, UNRELATE PATH '/**'
            )"#;
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::CreateRole(cr) => {
                assert_eq!(cr.permissions.len(), 2);
                assert_eq!(cr.permissions[0].operations.len(), 4);
                assert_eq!(
                    cr.permissions[0].operations,
                    vec![
                        Operation::Create,
                        Operation::Read,
                        Operation::Update,
                        Operation::Delete
                    ]
                );
                assert_eq!(cr.permissions[1].operations.len(), 2);
                assert_eq!(
                    cr.permissions[1].operations,
                    vec![Operation::Relate, Operation::Unrelate]
                );
            }
            other => panic!("Expected CreateRole, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_role_add_permission() {
        let sql = "ALTER ROLE 'editor' ADD PERMISSION ALLOW READ PATH '/content/**'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterRole(ar) => {
                assert_eq!(ar.role_id, "editor");
                match &ar.action {
                    AlterRoleAction::AddPermission(p) => {
                        assert_eq!(p.operations, vec![Operation::Read]);
                        assert_eq!(p.path, "/content/**");
                    }
                    other => panic!("Expected AddPermission, got {:?}", other),
                }
            }
            other => panic!("Expected AlterRole, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_role_drop_permission() {
        let sql = "ALTER ROLE 'editor' DROP PERMISSION 2";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterRole(ar) => {
                assert_eq!(ar.role_id, "editor");
                assert!(matches!(ar.action, AlterRoleAction::DropPermission(2)));
            }
            other => panic!("Expected AlterRole, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_role_add_inherits() {
        let sql = "ALTER ROLE 'editor' ADD INHERITS ('viewer', 'commenter')";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterRole(ar) => {
                assert_eq!(ar.role_id, "editor");
                match &ar.action {
                    AlterRoleAction::AddInherits(roles) => {
                        assert_eq!(roles, &vec!["viewer".to_string(), "commenter".to_string()]);
                    }
                    other => panic!("Expected AddInherits, got {:?}", other),
                }
            }
            other => panic!("Expected AlterRole, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_role_set_description() {
        let sql = "ALTER ROLE 'editor' SET DESCRIPTION 'Updated desc'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterRole(ar) => {
                assert_eq!(ar.role_id, "editor");
                match &ar.action {
                    AlterRoleAction::SetDescription(d) => assert_eq!(d, "Updated desc"),
                    other => panic!("Expected SetDescription, got {:?}", other),
                }
            }
            other => panic!("Expected AlterRole, got {:?}", other),
        }
    }

    #[test]
    fn test_drop_role() {
        let stmt = parse_acl("DROP ROLE 'editor'").unwrap().unwrap();
        match stmt {
            AclStatement::DropRole(dr) => {
                assert_eq!(dr.role_id, "editor");
                assert!(!dr.if_exists);
            }
            other => panic!("Expected DropRole, got {:?}", other),
        }
    }

    #[test]
    fn test_drop_role_if_exists() {
        let stmt = parse_acl("DROP ROLE IF EXISTS 'editor'").unwrap().unwrap();
        match stmt {
            AclStatement::DropRole(dr) => {
                assert_eq!(dr.role_id, "editor");
                assert!(dr.if_exists);
            }
            other => panic!("Expected DropRole, got {:?}", other),
        }
    }

    #[test]
    fn test_show_roles() {
        let stmt = parse_acl("SHOW ROLES").unwrap().unwrap();
        match stmt {
            AclStatement::ShowRoles(sr) => assert!(sr.like_pattern.is_none()),
            other => panic!("Expected ShowRoles, got {:?}", other),
        }
    }

    #[test]
    fn test_show_roles_like() {
        let stmt = parse_acl("SHOW ROLES LIKE 'content-%'").unwrap().unwrap();
        match stmt {
            AclStatement::ShowRoles(sr) => {
                assert_eq!(sr.like_pattern.as_deref(), Some("content-%"));
            }
            other => panic!("Expected ShowRoles, got {:?}", other),
        }
    }

    #[test]
    fn test_describe_role() {
        let stmt = parse_acl("DESCRIBE ROLE 'editor'").unwrap().unwrap();
        match stmt {
            AclStatement::DescribeRole(dr) => assert_eq!(dr.role_id, "editor"),
            other => panic!("Expected DescribeRole, got {:?}", other),
        }
    }

    // =========================================================================
    // Group tests
    // =========================================================================

    #[test]
    fn test_create_group() {
        let sql = "CREATE GROUP 'editors' DESCRIPTION 'Editor group' ROLES ('editor', 'reviewer')";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::CreateGroup(cg) => {
                assert_eq!(cg.group_id, "editors");
                assert_eq!(cg.description.as_deref(), Some("Editor group"));
                assert_eq!(cg.roles, vec!["editor", "reviewer"]);
            }
            other => panic!("Expected CreateGroup, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_group_add_roles() {
        let sql = "ALTER GROUP 'editors' ADD ROLES ('admin', 'moderator')";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterGroup(ag) => {
                assert_eq!(ag.group_id, "editors");
                match &ag.action {
                    AlterGroupAction::AddRoles(r) => {
                        assert_eq!(r, &vec!["admin".to_string(), "moderator".to_string()]);
                    }
                    other => panic!("Expected AddRoles, got {:?}", other),
                }
            }
            other => panic!("Expected AlterGroup, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_group_drop_roles() {
        let sql = "ALTER GROUP 'editors' DROP ROLES ('viewer')";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterGroup(ag) => {
                assert_eq!(ag.group_id, "editors");
                match &ag.action {
                    AlterGroupAction::DropRoles(r) => {
                        assert_eq!(r, &vec!["viewer".to_string()]);
                    }
                    other => panic!("Expected DropRoles, got {:?}", other),
                }
            }
            other => panic!("Expected AlterGroup, got {:?}", other),
        }
    }

    #[test]
    fn test_drop_group() {
        let stmt = parse_acl("DROP GROUP 'editors'").unwrap().unwrap();
        match stmt {
            AclStatement::DropGroup(dg) => {
                assert_eq!(dg.group_id, "editors");
                assert!(!dg.if_exists);
            }
            other => panic!("Expected DropGroup, got {:?}", other),
        }
    }

    #[test]
    fn test_show_groups() {
        let stmt = parse_acl("SHOW GROUPS").unwrap().unwrap();
        match stmt {
            AclStatement::ShowGroups(sg) => assert!(sg.like_pattern.is_none()),
            other => panic!("Expected ShowGroups, got {:?}", other),
        }
    }

    #[test]
    fn test_describe_group() {
        let stmt = parse_acl("DESCRIBE GROUP 'editors'").unwrap().unwrap();
        match stmt {
            AclStatement::DescribeGroup(dg) => assert_eq!(dg.group_id, "editors"),
            other => panic!("Expected DescribeGroup, got {:?}", other),
        }
    }

    // =========================================================================
    // User tests
    // =========================================================================

    #[test]
    fn test_create_user_minimal() {
        let sql = "CREATE USER 'alice' EMAIL 'alice@example.com'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::CreateUser(cu) => {
                assert_eq!(cu.user_id, "alice");
                assert_eq!(cu.email, "alice@example.com");
                assert!(cu.display_name.is_none());
                assert!(cu.roles.is_empty());
                assert!(cu.groups.is_empty());
                assert!(cu.can_login.is_none());
                assert!(cu.birth_date.is_none());
                assert!(cu.folder.is_none());
            }
            other => panic!("Expected CreateUser, got {:?}", other),
        }
    }

    #[test]
    fn test_create_user_full() {
        let sql = r#"CREATE USER 'bob' EMAIL 'bob@example.com'
            DISPLAY NAME 'Bob Smith'
            ROLES ('editor', 'viewer')
            GROUPS ('team-a')
            CAN LOGIN true
            BIRTH DATE '1990-01-15'
            IN FOLDER '/users/team-a'"#;
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::CreateUser(cu) => {
                assert_eq!(cu.user_id, "bob");
                assert_eq!(cu.email, "bob@example.com");
                assert_eq!(cu.display_name.as_deref(), Some("Bob Smith"));
                assert_eq!(cu.roles, vec!["editor", "viewer"]);
                assert_eq!(cu.groups, vec!["team-a"]);
                assert_eq!(cu.can_login, Some(true));
                assert_eq!(cu.birth_date.as_deref(), Some("1990-01-15"));
                assert_eq!(cu.folder.as_deref(), Some("/users/team-a"));
            }
            other => panic!("Expected CreateUser, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_user_add_roles() {
        let sql = "ALTER USER 'alice' ADD ROLES ('editor', 'reviewer')";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterUser(au) => {
                assert_eq!(au.user_id, "alice");
                match &au.action {
                    AlterUserAction::AddRoles(r) => {
                        assert_eq!(r, &vec!["editor".to_string(), "reviewer".to_string()]);
                    }
                    other => panic!("Expected AddRoles, got {:?}", other),
                }
            }
            other => panic!("Expected AlterUser, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_user_set_email() {
        let sql = "ALTER USER 'alice' SET EMAIL 'new@example.com'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterUser(au) => {
                assert_eq!(au.user_id, "alice");
                match &au.action {
                    AlterUserAction::SetEmail(e) => assert_eq!(e, "new@example.com"),
                    other => panic!("Expected SetEmail, got {:?}", other),
                }
            }
            other => panic!("Expected AlterUser, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_user_set_can_login() {
        let sql = "ALTER USER 'alice' SET CAN LOGIN false";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterUser(au) => {
                assert_eq!(au.user_id, "alice");
                match &au.action {
                    AlterUserAction::SetCanLogin(val) => assert!(!val),
                    other => panic!("Expected SetCanLogin, got {:?}", other),
                }
            }
            other => panic!("Expected AlterUser, got {:?}", other),
        }
    }

    #[test]
    fn test_drop_user() {
        let stmt = parse_acl("DROP USER 'alice'").unwrap().unwrap();
        match stmt {
            AclStatement::DropUser(du) => {
                assert_eq!(du.user_id, "alice");
                assert!(!du.if_exists);
            }
            other => panic!("Expected DropUser, got {:?}", other),
        }
    }

    #[test]
    fn test_show_users() {
        let stmt = parse_acl("SHOW USERS").unwrap().unwrap();
        match stmt {
            AclStatement::ShowUsers(su) => {
                assert!(su.like_pattern.is_none());
                assert!(su.in_group.is_none());
                assert!(su.with_role.is_none());
            }
            other => panic!("Expected ShowUsers, got {:?}", other),
        }
    }

    #[test]
    fn test_show_users_in_group() {
        let sql = "SHOW USERS IN GROUP 'engineering'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::ShowUsers(su) => {
                assert_eq!(su.in_group.as_deref(), Some("engineering"));
                assert!(su.with_role.is_none());
            }
            other => panic!("Expected ShowUsers, got {:?}", other),
        }
    }

    #[test]
    fn test_show_users_with_role() {
        let sql = "SHOW USERS WITH ROLE 'editor'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::ShowUsers(su) => {
                assert!(su.in_group.is_none());
                assert_eq!(su.with_role.as_deref(), Some("editor"));
            }
            other => panic!("Expected ShowUsers, got {:?}", other),
        }
    }

    #[test]
    fn test_describe_user() {
        let stmt = parse_acl("DESCRIBE USER 'alice'").unwrap().unwrap();
        match stmt {
            AclStatement::DescribeUser(du) => assert_eq!(du.user_id, "alice"),
            other => panic!("Expected DescribeUser, got {:?}", other),
        }
    }

    // =========================================================================
    // Grant / Revoke tests
    // =========================================================================

    #[test]
    fn test_grant_role_to_user() {
        let sql = "GRANT ROLE 'editor' TO USER 'alice'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::Grant(g) => {
                assert!(matches!(&g.target, GrantTarget::User(u) if u == "alice"));
                assert_eq!(g.grants.len(), 1);
                assert!(matches!(&g.grants[0], GrantItem::Role(r) if r == "editor"));
            }
            other => panic!("Expected Grant, got {:?}", other),
        }
    }

    #[test]
    fn test_grant_roles_to_user() {
        let sql = "GRANT ROLES ('editor', 'viewer') TO USER 'alice'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::Grant(g) => {
                assert!(matches!(&g.target, GrantTarget::User(u) if u == "alice"));
                assert_eq!(g.grants.len(), 2);
                assert!(matches!(&g.grants[0], GrantItem::Role(r) if r == "editor"));
                assert!(matches!(&g.grants[1], GrantItem::Role(r) if r == "viewer"));
            }
            other => panic!("Expected Grant, got {:?}", other),
        }
    }

    #[test]
    fn test_grant_group_to_user() {
        let sql = "GRANT GROUP 'engineering' TO USER 'alice'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::Grant(g) => {
                assert!(matches!(&g.target, GrantTarget::User(u) if u == "alice"));
                assert_eq!(g.grants.len(), 1);
                assert!(matches!(&g.grants[0], GrantItem::Group(g) if g == "engineering"));
            }
            other => panic!("Expected Grant, got {:?}", other),
        }
    }

    #[test]
    fn test_grant_role_to_group() {
        let sql = "GRANT ROLE 'deployer' TO GROUP 'engineering'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::Grant(g) => {
                assert!(matches!(&g.target, GrantTarget::Group(g) if g == "engineering"));
                assert_eq!(g.grants.len(), 1);
                assert!(matches!(&g.grants[0], GrantItem::Role(r) if r == "deployer"));
            }
            other => panic!("Expected Grant, got {:?}", other),
        }
    }

    #[test]
    fn test_revoke_role_from_user() {
        let sql = "REVOKE ROLE 'editor' FROM USER 'alice'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::Revoke(r) => {
                assert!(matches!(&r.target, RevokeTarget::User(u) if u == "alice"));
                assert_eq!(r.revocations.len(), 1);
                assert!(matches!(&r.revocations[0], RevokeItem::Role(r) if r == "editor"));
            }
            other => panic!("Expected Revoke, got {:?}", other),
        }
    }

    #[test]
    fn test_revoke_group_from_user() {
        let sql = "REVOKE GROUP 'engineering' FROM USER 'alice'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::Revoke(r) => {
                assert!(matches!(&r.target, RevokeTarget::User(u) if u == "alice"));
                assert_eq!(r.revocations.len(), 1);
                assert!(matches!(&r.revocations[0], RevokeItem::Group(g) if g == "engineering"));
            }
            other => panic!("Expected Revoke, got {:?}", other),
        }
    }

    // =========================================================================
    // Security config tests
    // =========================================================================

    #[test]
    fn test_alter_security_config() {
        let sql = r#"ALTER SECURITY CONFIG '*'
            SET DEFAULT POLICY 'deny-all'
            SET ANONYMOUS ENABLED true
            SET ANONYMOUS ROLE 'public-reader'"#;
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterSecurityConfig(asc) => {
                assert_eq!(asc.workspace_pattern, "*");
                assert_eq!(asc.settings.len(), 3);
                assert!(matches!(
                    &asc.settings[0],
                    SecurityConfigSetting::DefaultPolicy(p) if p == "deny-all"
                ));
                assert!(matches!(
                    &asc.settings[1],
                    SecurityConfigSetting::AnonymousEnabled(true)
                ));
                assert!(matches!(
                    &asc.settings[2],
                    SecurityConfigSetting::AnonymousRole(r) if r == "public-reader"
                ));
            }
            other => panic!("Expected AlterSecurityConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_alter_security_config_interface() {
        let sql = "ALTER SECURITY CONFIG 'ws' SET INTERFACE pgwire ANONYMOUS ENABLED true";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::AlterSecurityConfig(asc) => {
                assert_eq!(asc.workspace_pattern, "ws");
                assert_eq!(asc.settings.len(), 1);
                match &asc.settings[0] {
                    SecurityConfigSetting::InterfaceSetting {
                        interface,
                        key,
                        value,
                    } => {
                        assert_eq!(interface, "pgwire");
                        assert_eq!(key, "anonymous_enabled");
                        assert_eq!(value, "true");
                    }
                    other => panic!("Expected InterfaceSetting, got {:?}", other),
                }
            }
            other => panic!("Expected AlterSecurityConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_show_security_config() {
        let stmt = parse_acl("SHOW SECURITY CONFIG").unwrap().unwrap();
        match stmt {
            AclStatement::ShowSecurityConfig(ssc) => assert!(ssc.workspace.is_none()),
            other => panic!("Expected ShowSecurityConfig, got {:?}", other),
        }
    }

    #[test]
    fn test_show_security_config_for() {
        let stmt = parse_acl("SHOW SECURITY CONFIG FOR 'content'")
            .unwrap()
            .unwrap();
        match stmt {
            AclStatement::ShowSecurityConfig(ssc) => {
                assert_eq!(ssc.workspace.as_deref(), Some("content"));
            }
            other => panic!("Expected ShowSecurityConfig, got {:?}", other),
        }
    }

    // =========================================================================
    // Introspection tests
    // =========================================================================

    #[test]
    fn test_show_permissions_for() {
        let sql = "SHOW PERMISSIONS FOR USER 'alice'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::ShowPermissionsFor(spf) => {
                assert_eq!(spf.user_id, "alice");
                assert!(spf.workspace.is_none());
            }
            other => panic!("Expected ShowPermissionsFor, got {:?}", other),
        }
    }

    #[test]
    fn test_show_permissions_for_on() {
        let sql = "SHOW PERMISSIONS FOR USER 'alice' ON 'content'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::ShowPermissionsFor(spf) => {
                assert_eq!(spf.user_id, "alice");
                assert_eq!(spf.workspace.as_deref(), Some("content"));
            }
            other => panic!("Expected ShowPermissionsFor, got {:?}", other),
        }
    }

    #[test]
    fn test_show_effective_roles_for() {
        let sql = "SHOW EFFECTIVE ROLES FOR USER 'alice'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::ShowEffectiveRolesFor(ser) => {
                assert_eq!(ser.user_id, "alice");
            }
            other => panic!("Expected ShowEffectiveRolesFor, got {:?}", other),
        }
    }

    // =========================================================================
    // Guard function tests
    // =========================================================================

    #[test]
    fn test_is_acl_statement_positive() {
        // Role statements
        assert!(is_acl_statement("CREATE ROLE 'editor'"));
        assert!(is_acl_statement("ALTER ROLE 'editor' SET DESCRIPTION 'x'"));
        assert!(is_acl_statement("DROP ROLE 'editor'"));
        assert!(is_acl_statement("SHOW ROLES"));
        assert!(is_acl_statement("DESCRIBE ROLE 'editor'"));
        // Group statements
        assert!(is_acl_statement("CREATE GROUP 'editors'"));
        assert!(is_acl_statement("ALTER GROUP 'editors' ADD ROLES ('a')"));
        assert!(is_acl_statement("DROP GROUP 'editors'"));
        assert!(is_acl_statement("SHOW GROUPS"));
        assert!(is_acl_statement("DESCRIBE GROUP 'editors'"));
        // User statements
        assert!(is_acl_statement("CREATE USER 'alice' EMAIL 'a@b.com'"));
        assert!(is_acl_statement("ALTER USER 'alice' SET EMAIL 'new@b.com'"));
        assert!(is_acl_statement("DROP USER 'alice'"));
        assert!(is_acl_statement("SHOW USERS"));
        assert!(is_acl_statement("DESCRIBE USER 'alice'"));
        // Grant / Revoke
        assert!(is_acl_statement("GRANT ROLE 'editor' TO USER 'alice'"));
        assert!(is_acl_statement("REVOKE ROLE 'editor' FROM USER 'alice'"));
        // Security config
        assert!(is_acl_statement(
            "ALTER SECURITY CONFIG 'ws' SET DEFAULT POLICY 'deny'"
        ));
        assert!(is_acl_statement("SHOW SECURITY CONFIG"));
        assert!(is_acl_statement("SHOW SECURITY CONFIG FOR 'myws'"));
        // Introspection
        assert!(is_acl_statement("SHOW PERMISSIONS FOR USER 'alice'"));
        assert!(is_acl_statement("SHOW EFFECTIVE ROLES FOR USER 'alice'"));
    }

    #[test]
    fn test_is_acl_statement_negative() {
        assert!(!is_acl_statement("SELECT * FROM nodes"));
        assert!(!is_acl_statement("SHOW BRANCHES"));
        assert!(!is_acl_statement("ORDER BY name"));
        assert!(!is_acl_statement("CREATE TABLE foo (id INT)"));
        assert!(!is_acl_statement("INSERT INTO nodes VALUES (1)"));
        assert!(!is_acl_statement("SHOW CURRENT BRANCH"));
        assert!(!is_acl_statement("DELETE FROM nodes WHERE id = 1"));
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_case_insensitivity() {
        // Mixed case should parse correctly
        let stmt = parse_acl("create ROLE 'viewer'").unwrap().unwrap();
        assert!(matches!(stmt, AclStatement::CreateRole(_)));

        let stmt = parse_acl("Grant Role 'editor' To User 'alice'")
            .unwrap()
            .unwrap();
        assert!(matches!(stmt, AclStatement::Grant(_)));

        let stmt = parse_acl("show ROLES").unwrap().unwrap();
        assert!(matches!(stmt, AclStatement::ShowRoles(_)));

        let stmt = parse_acl("Describe Group 'editors'").unwrap().unwrap();
        assert!(matches!(stmt, AclStatement::DescribeGroup(_)));
    }

    #[test]
    fn test_trailing_semicolon() {
        let stmt = parse_acl("SHOW ROLES;").unwrap().unwrap();
        assert!(matches!(stmt, AclStatement::ShowRoles(_)));

        let stmt = parse_acl("DROP ROLE 'editor';").unwrap().unwrap();
        assert!(matches!(stmt, AclStatement::DropRole(_)));

        let stmt = parse_acl("GRANT ROLE 'editor' TO USER 'alice';")
            .unwrap()
            .unwrap();
        assert!(matches!(stmt, AclStatement::Grant(_)));
    }

    #[test]
    fn test_extra_whitespace() {
        // Leading/trailing whitespace
        let stmt = parse_acl("  SHOW ROLES  ").unwrap().unwrap();
        assert!(matches!(stmt, AclStatement::ShowRoles(_)));

        // Leading whitespace with trailing semicolon
        let stmt = parse_acl("   DROP ROLE 'editor'  ;  ").unwrap().unwrap();
        match stmt {
            AclStatement::DropRole(dr) => assert_eq!(dr.role_id, "editor"),
            other => panic!("Expected DropRole, got {:?}", other),
        }

        // Whitespace before semicolon
        let stmt = parse_acl("SHOW GROUPS   ;  ").unwrap().unwrap();
        assert!(matches!(stmt, AclStatement::ShowGroups(_)));

        // Multiline CREATE ROLE with extra whitespace in body
        let sql = "CREATE ROLE 'viewer'\n    DESCRIPTION   'A viewer role'";
        let stmt = parse_acl(sql).unwrap().unwrap();
        match stmt {
            AclStatement::CreateRole(cr) => {
                assert_eq!(cr.role_id, "viewer");
                assert_eq!(cr.description.as_deref(), Some("A viewer role"));
            }
            other => panic!("Expected CreateRole, got {:?}", other),
        }
    }

    #[test]
    fn test_non_acl_returns_none() {
        let result = parse_acl("SELECT * FROM nodes").unwrap();
        assert!(result.is_none());
    }
}
