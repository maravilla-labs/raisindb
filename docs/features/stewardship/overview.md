# Stewardship System Overview

## What is Stewardship?

The RaisinDB Stewardship System enables authorized users (Stewards) to act on behalf of other users (Wards) in your application. This creates a secure delegation model where one person can manage accounts, data, and activities for others who cannot or should not manage these themselves.

Common examples include:
- Parents managing their children's accounts
- Legal guardians overseeing dependents
- Executive assistants handling tasks for executives
- Temporary coverage during vacations or leave

## Why Stewardship?

Many applications need to support scenarios where users require assistance from trusted representatives. Traditional access control focuses on what individual users can do, but stewardship addresses who can act for whom.

The stewardship system provides:
- **Clear accountability**: All actions performed by stewards are tracked with both the steward's and ward's identities
- **Flexible configurations**: Support for different types of relationships and delegation scopes
- **Safety controls**: Configurable limits on steward/ward relationships and time-bound delegations
- **Consent workflows**: Optional ward approval processes for establishing stewardship

## Key Concepts

### Steward
A **Steward** is a user who has been authorized to act on behalf of one or more Wards. When a steward performs actions, they can do so in the ward's context while maintaining clear audit trails showing who actually performed the action.

Example: Maria is a steward for her two children, allowing her to manage their school registrations and activity sign-ups.

### Ward
A **Ward** is a user who has delegated authority to one or more Stewards. Wards may be minors, individuals with special needs, or anyone who has chosen to delegate certain responsibilities.

Example: Emma (age 10) is a ward of her mother Maria and father James, who manage her account until she reaches adulthood.

### RelationType
**RelationTypes** define the types of relationships that can exist between users in your system. Certain relation types can automatically imply stewardship.

The system includes common relation types out of the box:
- **PARENT_OF / CHILD_OF**: Parent-child relationships (implies stewardship when child is a minor)
- **GUARDIAN_OF / WARD_OF**: Legal guardianship relationships (always implies stewardship)
- **MANAGER_OF / REPORTS_TO**: Organizational hierarchy
- **HAS_ASSISTANT / ASSISTANT_OF**: Executive-assistant relationships
- **SPOUSE_OF**: Marital relationships
- **SIBLING_OF**: Sibling relationships
- **GRANDPARENT_OF / GRANDCHILD_OF**: Extended family

You can configure which relation types imply stewardship and whether they require the ward to be a minor.

### EntityCircle
**EntityCircles** are groupings of users who share a common context, such as families, teams, or organizational units. They help organize related users and can be referenced in permissions and access control rules.

Examples:
- The "Smith Family" circle containing parents and children
- The "Engineering Team" circle containing team members
- The "Executive Suite" circle containing executives and assistants

### Minor Status
Users below a configurable age threshold (default: 18 years) are considered minors. Minor status affects:
- Which relation types grant stewardship (e.g., PARENT_OF typically only grants stewardship for minors)
- Whether ward consent is required
- Whether the user can log in directly

### Stewardship Derivation

Stewardship can be established in two ways:

#### 1. Relationship-Based (Primary Method)
Stewardship is automatically derived from graph relationships between users. When a relationship exists that matches a configured stewardship relation type, stewardship is automatically granted.

**How it works:**
1. User A establishes a relationship with User B using a relation type (e.g., PARENT_OF)
2. The system checks if that relation type implies stewardship
3. If yes, User A becomes a steward of User B
4. Additional checks may apply (e.g., whether User B is a minor)

**Example:**
```
John --[PARENT_OF]--> Emma (age 12)

Since PARENT_OF implies stewardship and Emma is a minor:
Result: John is a steward of Emma
```

#### 2. Override-Based (Manual Assignment)
For special cases, administrators or authorized users can create explicit StewardshipOverride records. These are useful for:
- Temporary delegations with specific time limits
- Scoped delegations limited to certain permissions
- Situations where no natural relationship exists

## Delegation Types

### Full Delegation
With full delegation, the steward can perform any action the ward could perform. This is the default mode for relationship-based stewardship.

**Use case:** A parent managing a young child's account has full access to all features.

### Scoped Delegation
With scoped delegation, the steward can only perform specific actions defined in the StewardshipOverride. This provides fine-grained control for temporary or limited access scenarios.

**Use case:** An assistant authorized to schedule meetings and manage calendar, but not access financial records.

## Time-Limited Delegations

StewardshipOverrides can include start and end dates, allowing temporary access that automatically expires. This is useful for:
- Vacation coverage
- Project-based assistance
- Trial periods

**Example:**
```
valid_from: 2025-01-01
valid_until: 2025-01-15
Status changes from "active" to "expired" automatically after end date
```

## Workflows for Establishing Stewardship

The system supports three configurable workflows:

### 1. Invitation Workflow
The ward initiates by sending an invitation to a potential steward.
1. Ward sends stewardship invitation
2. Potential steward receives and reviews invitation
3. Steward accepts or declines
4. Upon acceptance, relationship is created and stewardship begins

### 2. Admin Assignment
An administrator directly establishes the stewardship relationship.
1. Admin creates the relationship or StewardshipOverride
2. Both parties are notified
3. Stewardship is immediately active

### 3. Steward Creates Ward
The steward creates a new user account that automatically becomes their ward.
1. Steward initiates ward account creation
2. New user account is created
3. Stewardship relationship is automatically established
4. Steward can manage the new account

This workflow must be explicitly enabled in configuration.

## What's Included Out of the Box

The RaisinDB Stewardship Package (`raisin-stewardship`) provides:

### Node Types
- **StewardshipConfig**: Repository-level configuration
- **RelationType**: Relationship type definitions
- **EntityCircle**: User grouping containers
- **StewardshipOverride**: Manual delegation records

### Pre-configured Relation Types
- Household relationships (parent/child, guardian/ward, spouse, sibling, grandparent/grandchild)
- Organization relationships (manager/reports-to, assistant/has-assistant)

### Functions & Triggers
- Automatic relationship processing
- Invitation handling
- Stewardship resolution logic
- Permission checking

### Default Configuration
All settings have sensible defaults:
- Minor age threshold: 18 years
- Max stewards per ward: 5
- Max wards per steward: 10
- Invitation expiry: 7 days
- Ward consent required: Yes
- Stewardship disabled by default (must be enabled per repository)

## Security & Privacy Considerations

### Audit Trail
All actions performed by stewards are logged with both identities, ensuring accountability and compliance.

### Consent Requirements
By default, wards must consent to stewardship relationships (except for minors, who may not be able to provide meaningful consent).

### Configurable Limits
Administrators can configure maximum numbers of stewards per ward and wards per steward to prevent abuse.

### Revocation
Stewardship relationships can be revoked at any time by:
- The ward (if they have capacity)
- The steward (voluntarily relinquishing authority)
- An administrator
- Automatic expiration (for time-limited delegations)

## Integration with Access Control

The stewardship system integrates with RaisinDB's Row-Level Security (RLS) and permission system:

- When a steward acts as a ward, they inherit the ward's permissions
- Steward actions are subject to the same RLS filters as the ward
- The system maintains context about both the steward and ward identities
- Permission checks evaluate whether the steward is authorized to act on behalf of the ward

## Next Steps

- **For common usage scenarios**: See [Use Cases](./use-cases.md)
- **To configure stewardship in your repository**: See [Configuration Guide](./configuration.md)
- **For term definitions**: See [Glossary](./glossary.md)
- **For technical implementation details**: See developer documentation in `/docs/developer/stewardship/`
