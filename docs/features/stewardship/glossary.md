# Stewardship System Glossary

This glossary defines all key terms used in the RaisinDB Stewardship System. Terms are organized alphabetically with cross-references.

---

## A

### Action
An operation performed by a user or steward within the system, such as creating a record, updating data, or approving a request. When performed by a steward, actions are recorded with both the steward's and ward's identities for audit purposes.

**Related terms:** Audit Trail, Steward

### Admin Assignment
A workflow for establishing stewardship where a system administrator directly creates the relationship or StewardshipOverride without requiring invitation or consent from either party. Commonly used for verified legal guardianships or employment relationships.

**Related terms:** Workflow, Stewardship Override, Invitation Workflow

### Adult
A user who is at or above the minor age threshold. Adults have full control over their accounts and can independently consent to stewardship relationships.

**Related terms:** Minor, Minor Age Threshold

### Audit Trail
A complete log of all actions performed in the system, including those performed by stewards on behalf of wards. Audit trails record both the steward's identity (who actually performed the action) and the ward's identity (on whose behalf the action was performed).

**Related terms:** Action, Accountability, Steward

---

## B

### Bidirectional Relationship
A relationship type that automatically exists in both directions when created. For example, if SPOUSE_OF is bidirectional and John is spouse of Maria, then Maria is automatically spouse of John.

**Related terms:** RelationType, Inverse Relation

---

## C

### Consent
Explicit approval from a ward to establish a stewardship relationship. The system can be configured to require ward consent for new stewardships, though this may be bypassed for minors or in admin assignment workflows.

**Related terms:** Ward, Require Ward Consent, Invitation Workflow

### Care Coordinator
A user role (typically in healthcare contexts) responsible for managing aspects of a patient's care. Care coordinators typically have scoped delegations to manage appointments and care plans but not financial or highly sensitive information.

**Related terms:** Scoped Delegation, Healthcare Use Case

---

## D

### Delegation
The act of granting authority to a steward to act on behalf of a ward. Delegations can be full (all permissions) or scoped (specific permissions only).

**Related terms:** Full Delegation, Scoped Delegation, Steward, Ward

### Delegation Mode
The type of delegation granted in a StewardshipOverride. Can be either "full" (steward can perform any action the ward could) or "scoped" (steward can only perform specific actions defined in scoped_permissions).

**Related terms:** Full Delegation, Scoped Delegation, Stewardship Override

---

## E

### EntityCircle
A node type representing a group of users who share a common context. EntityCircles can represent families, teams, departments, or any other logical grouping. They help organize related users and can be referenced in permissions and access control rules.

**Properties:**
- name: Display name (e.g., "Smith Family")
- circle_type: Type of circle (e.g., "family", "team", "org_unit")
- primary_contact_id: User ID of the primary contact
- address: Shared address information
- metadata: Extensible properties

**Related terms:** Family, Team, Organization

### Expiration
The automatic ending of a time-limited stewardship. When a StewardshipOverride reaches its valid_until date, its status automatically changes to "expired" and the steward loses access.

**Related terms:** Time-Limited Delegation, Valid Until, Stewardship Override

---

## F

### Family
An EntityCircle representing a household unit. Typically includes parents, children, and possibly extended family members. The stewardship system provides specific support for family scenarios through the PARENT_OF and GUARDIAN_OF relation types.

**Related terms:** EntityCircle, Household, PARENT_OF, GUARDIAN_OF

### Full Delegation
A delegation mode where the steward can perform any action the ward could perform. This is the default mode for relationship-based stewardship and provides complete access within the ward's permission scope.

**Contrast with:** Scoped Delegation

**Related terms:** Delegation Mode, Steward, Ward

---

## G

### Guardian
A user who has legal or assigned responsibility for another user (the ward). In the stewardship system, guardians are stewards with full or scoped delegations, typically established through the GUARDIAN_OF relation type.

**Related terms:** Steward, Ward, GUARDIAN_OF, Legal Guardianship

### GUARDIAN_OF
A RelationType representing a legal or assigned guardianship relationship. This relation type implies stewardship and does not require the ward to be a minor (unlike PARENT_OF). The inverse relation is WARD_OF.

**Properties:**
- implies_stewardship: true
- requires_minor: false

**Related terms:** RelationType, Stewardship, Guardian, Ward

---

## H

### HAS_ASSISTANT
A RelationType representing the relationship from an executive or principal to their assistant. The inverse relation is ASSISTANT_OF. By default, this relation does not imply stewardship but can be configured to do so in organizational settings.

**Related terms:** RelationType, Assistant, Organization

### Household
A group of people living together and sharing resources, typically represented as a family-type EntityCircle. Household scenarios are a primary use case for the stewardship system.

**Related terms:** Family, EntityCircle, PARENT_OF

---

## I

### Invitation Workflow
A workflow for establishing stewardship where the ward initiates by sending an invitation to a potential steward, or a potential steward requests access from the ward. The recipient must accept the invitation for stewardship to be established. Requires ward consent.

**Steps:**
1. Ward or steward initiates invitation
2. Recipient receives notification
3. Recipient accepts or declines
4. Upon acceptance, relationship is created and stewardship begins

**Related terms:** Workflow, Admin Assignment, Consent

### Inverse Relation
The opposite direction of a relationship. For example, CHILD_OF is the inverse of PARENT_OF. When a relationship is created in one direction, the system can automatically create the inverse relationship.

**Related terms:** RelationType, Bidirectional Relationship

---

## L

### Legal Guardianship
A formal legal arrangement where one person is granted authority to make decisions for another. In the stewardship system, legal guardianships are typically represented through GUARDIAN_OF relationships and established via admin assignment after verification of legal documentation.

**Related terms:** Guardian, GUARDIAN_OF, Admin Assignment

---

## M

### MANAGER_OF
A RelationType representing an organizational reporting relationship. The inverse relation is REPORTS_TO. By default, this relation does not imply stewardship but can be configured to do so for organizational delegation scenarios.

**Related terms:** RelationType, Organization, Reporting Structure

### Minor
A user below the configured minor age threshold. Minors typically cannot consent to stewardship relationships and may not be able to log in directly depending on configuration. The PARENT_OF relation type only grants stewardship when the target is a minor (if require_minor_for_parent is enabled).

**Related terms:** Minor Age Threshold, Minor Status, Adult

### Minor Age Threshold
The age below which a user is considered a minor. This is a configurable setting in StewardshipConfig with a default value of 18. The threshold affects stewardship derivation, consent requirements, and login permissions.

**Configuration:** `minor_age_threshold` in StewardshipConfig

**Related terms:** Minor, Minor Status, PARENT_OF

### Minor Status
A boolean indicator of whether a user is below the minor age threshold. Minor status is automatically calculated based on the user's date of birth and the configured minor age threshold. It affects which relation types grant stewardship and whether consent is required.

**Related terms:** Minor, Minor Age Threshold

---

## O

### Organization
An EntityCircle representing a business, department, team, or other organizational unit. Organizations can use stewardship for delegation scenarios such as assistants managing executive accounts or managers overseeing team members.

**Related terms:** EntityCircle, Team, MANAGER_OF, HAS_ASSISTANT

---

## P

### PARENT_OF
A RelationType representing a parent-child relationship. This relation type implies stewardship when the child is a minor (if require_minor_for_parent is enabled). The inverse relation is CHILD_OF.

**Properties:**
- implies_stewardship: true
- requires_minor: true (by default)

**Related terms:** RelationType, Stewardship, Minor, CHILD_OF

### Permission
An authorization to perform a specific action on a specific resource. In scoped delegations, permissions define what actions the steward can perform on behalf of the ward.

**Example:**
```json
{
  "resource": "calendar",
  "actions": ["read", "write"]
}
```

**Related terms:** Scoped Delegation, Scoped Permissions

---

## R

### RelationType
A node type representing a type of relationship that can exist between users. RelationTypes define how relationships appear in the UI, whether they imply stewardship, and whether stewardship requires the ward to be a minor.

**Properties:**
- relation_name: Graph relation type (e.g., "PARENT_OF")
- title: Display name
- description: Human-readable description
- category: Grouping category (e.g., "household", "organization")
- inverse_relation_name: Inverse relation type
- bidirectional: Whether relation exists both ways automatically
- implies_stewardship: Whether this relation grants stewardship
- requires_minor: Whether stewardship requires ward to be minor
- icon: UI icon
- color: UI color

**Related terms:** Relationship, Stewardship, PARENT_OF, GUARDIAN_OF

### Relationship
A connection between two users in the system, defined by a RelationType. Relationships can be directional (e.g., PARENT_OF) or bidirectional (e.g., SPOUSE_OF). Certain relationships automatically grant stewardship.

**Related terms:** RelationType, Stewardship Derivation

### Relationship-Based Stewardship
Stewardship that is automatically derived from graph relationships between users. This is the primary method for establishing stewardship. When a relationship exists that matches a configured stewardship relation type, stewardship is automatically granted.

**Contrast with:** Override-Based Stewardship

**Related terms:** RelationType, Stewardship Derivation

### Require Minor for Parent
A configuration setting that determines whether PARENT_OF relationships only grant stewardship when the child is a minor. When enabled (default), parents lose stewardship when children reach the minor age threshold. When disabled, PARENT_OF always grants stewardship regardless of age.

**Configuration:** `require_minor_for_parent` in StewardshipConfig

**Related terms:** PARENT_OF, Minor Status, Minor Age Threshold

### Require Ward Consent
A configuration setting that determines whether wards must explicitly consent to stewardship relationships. When enabled (default), wards must accept invitations or approve stewardship. When disabled, stewardships can be established without ward action.

**Configuration:** `require_ward_consent` in StewardshipConfig

**Related terms:** Consent, Invitation Workflow

### Revocation
The act of ending a stewardship relationship. Revocation can be initiated by the ward, the steward, or an administrator. Upon revocation, the steward immediately loses the ability to act on behalf of the ward.

**Related terms:** Stewardship, Expiration

---

## S

### Scoped Delegation
A delegation mode where the steward can only perform specific actions defined in the scoped_permissions property of a StewardshipOverride. This provides fine-grained control for temporary or limited access scenarios.

**Example use case:** An assistant authorized to manage calendar and schedule meetings but not access financial records.

**Contrast with:** Full Delegation

**Related terms:** Delegation Mode, Scoped Permissions, Stewardship Override

### Scoped Permissions
An array of permission objects that define the specific actions a steward can perform in a scoped delegation. Each permission object specifies a resource type, allowed actions, and optional conditions.

**Structure:**
```json
[
  {
    "resource": "resource_type",
    "actions": ["action1", "action2"],
    "conditions": {
      "constraint": "value"
    }
  }
]
```

**Related terms:** Scoped Delegation, Permission

### Status
The current state of a StewardshipOverride. Possible values:
- **pending**: Stewardship is created but not yet active (awaiting start date or consent)
- **active**: Stewardship is currently active and steward can act as ward
- **expired**: Stewardship has reached its end date and is no longer active
- **revoked**: Stewardship was manually terminated before expiration

**Related terms:** Stewardship Override, Expiration, Revocation

### Steward
A user who has been authorized to act on behalf of one or more wards. When a steward performs actions, they do so in the ward's context while maintaining clear audit trails showing both identities.

**Related terms:** Ward, Stewardship, Delegation

### Steward Creates Ward
A workflow for establishing stewardship where the steward creates a new user account that automatically becomes their ward. The new account is created without normal registration and the stewardship relationship is immediately established.

**Requirements:**
- Must be enabled in allowed_workflows
- Must set steward_creates_ward_enabled: true

**Common use cases:** Parents creating children's accounts, HR onboarding employees

**Related terms:** Workflow, Invitation Workflow, Admin Assignment

### Stewardship
The relationship between a steward and a ward, granting the steward authority to act on behalf of the ward. Stewardship can be full (all permissions) or scoped (specific permissions only), and can be time-limited or indefinite.

**Related terms:** Steward, Ward, Delegation

### Stewardship Derivation
The process by which the system determines stewardship relationships. Stewardship can be derived from:
1. **Relationships** - Automatically from graph relationships with relation types that imply stewardship
2. **Overrides** - Explicitly through StewardshipOverride records

**Related terms:** Relationship-Based Stewardship, Override-Based Stewardship

### StewardshipConfig
A node type representing repository-level stewardship configuration settings. Each repository has one StewardshipConfig node that controls all aspects of the stewardship system.

**Properties:**
- enabled: Whether stewardship is enabled
- stewardship_relation_types: Which relation types imply stewardship
- require_minor_for_parent: Whether PARENT_OF only grants stewardship for minors
- allowed_workflows: Allowed workflows for establishing stewardship
- steward_creates_ward_enabled: Whether stewards can create ward accounts
- max_stewards_per_ward: Maximum stewards per ward
- max_wards_per_steward: Maximum wards per steward
- invitation_expiry_days: Days until invitation expires
- require_ward_consent: Whether ward consent is required
- minor_age_threshold: Age below which user is minor
- allow_minor_login: Whether minors can log in

**Related terms:** Configuration, Repository

### StewardshipOverride
A node type representing an explicit stewardship delegation that overrides or supplements relationship-based stewardship. Used for time-limited delegations, scoped delegations, or situations where no natural relationship exists.

**Properties:**
- steward_id: User ID of the steward
- ward_id: User ID of the ward
- delegation_mode: "full" or "scoped"
- scoped_permissions: Specific permissions (for scoped delegation)
- valid_from: Start date
- valid_until: End date (null = indefinite)
- status: "pending", "active", "expired", or "revoked"
- reason: Reason for the override

**Related terms:** Override-Based Stewardship, Scoped Delegation, Time-Limited Delegation

---

## T

### Team
An EntityCircle representing a work group or project team. Teams are a common use case for organizational stewardship scenarios.

**Related terms:** EntityCircle, Organization

### Time-Limited Delegation
A stewardship with a defined end date. Time-limited delegations automatically expire when they reach their valid_until date, at which point the steward loses access. Useful for vacation coverage, temporary assistance, or trial periods.

**Related terms:** Stewardship Override, Valid Until, Expiration

---

## V

### Valid From
The start date for a StewardshipOverride. If specified, the override remains in "pending" status until this date, then automatically becomes "active". If not specified, the override becomes active immediately upon creation.

**Related terms:** Stewardship Override, Valid Until, Time-Limited Delegation

### Valid Until
The end date for a StewardshipOverride. If specified, the override automatically changes to "expired" status on this date and the steward loses access. If null, the delegation is indefinite until manually revoked.

**Related terms:** Stewardship Override, Valid From, Time-Limited Delegation, Expiration

---

## W

### Ward
A user who has delegated authority to one or more stewards. Wards may be minors, individuals with special needs, or anyone who has chosen to delegate certain responsibilities. Depending on configuration, wards may or may not be able to log in directly.

**Related terms:** Steward, Stewardship, Delegation

### WARD_OF
A RelationType representing the inverse of GUARDIAN_OF. If User A is GUARDIAN_OF User B, then User B is WARD_OF User A.

**Related terms:** GUARDIAN_OF, RelationType

### Workflow
A process for establishing stewardship relationships. The system supports three workflows:
1. **Invitation** - Ward or steward initiates, requires consent
2. **Admin Assignment** - Administrator directly creates relationship
3. **Steward Creates Ward** - Steward creates new ward account

**Related terms:** Invitation Workflow, Admin Assignment, Steward Creates Ward

---

## Cross-Reference by Category

### User Roles
- Adult
- Guardian
- Minor
- Steward
- Ward

### Node Types
- EntityCircle
- RelationType
- StewardshipConfig
- StewardshipOverride

### Relationships
- Bidirectional Relationship
- GUARDIAN_OF
- HAS_ASSISTANT
- Inverse Relation
- MANAGER_OF
- PARENT_OF
- Relationship
- WARD_OF

### Delegation Concepts
- Delegation
- Delegation Mode
- Full Delegation
- Scoped Delegation
- Scoped Permissions
- Time-Limited Delegation

### Configuration
- Allow Minor Login
- Minor Age Threshold
- Require Minor for Parent
- Require Ward Consent
- StewardshipConfig

### Processes
- Admin Assignment
- Consent
- Expiration
- Invitation Workflow
- Revocation
- Steward Creates Ward
- Stewardship Derivation
- Workflow

### Organizational Concepts
- Care Coordinator
- EntityCircle
- Family
- Household
- Organization
- Team

### System Concepts
- Action
- Audit Trail
- Permission
- Status
- Valid From
- Valid Until

---

## Related Documentation

- **Overview:** [Stewardship System Overview](./overview.md)
- **Use Cases:** [Common Stewardship Scenarios](./use-cases.md)
- **Configuration:** [Admin Configuration Guide](./configuration.md)
- **Developer Documentation:** `/docs/developer/stewardship/`
