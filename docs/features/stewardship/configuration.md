# Stewardship Configuration Guide

This guide walks administrators through configuring and managing the stewardship system for their RaisinDB repository.

## Prerequisites

- Administrator access to the repository
- Access to the Admin Console or API
- Understanding of your organization's stewardship requirements (see [Use Cases](./use-cases.md))

## Quick Start

The stewardship system is **disabled by default**. To enable it:

1. Navigate to the Admin Console
2. Go to "Access Control" → "Stewardship Settings"
3. Toggle "Enable Stewardship" to ON
4. Configure settings based on your use case
5. Save changes

## Accessing Stewardship Configuration

### Through Admin Console

1. **Log in to Admin Console**
   - Navigate to your repository's admin console
   - Authenticate with admin credentials

2. **Navigate to Stewardship Settings**
   - In the left sidebar, find "Access Control"
   - Click on "Stewardship" or "Stewardship Settings"
   - You will see the StewardshipConfig editor

### Through API

Configuration is stored in the `raisin:StewardshipConfig` node at:
```
/raisin:access_control/config/stewardship
```

You can update it via the RaisinDB API:
```http
PATCH /api/repositories/{repo_id}/nodes/{node_id}
Content-Type: application/json

{
  "properties": {
    "enabled": true,
    "minor_age_threshold": 18
  }
}
```

## Configuration Settings

### Enable Stewardship

**Property:** `enabled`
**Type:** Boolean
**Default:** `false`

Enables or disables the entire stewardship system for the repository.

**When to enable:**
- You need parents to manage children's accounts
- Your organization requires delegation capabilities
- You have legal guardianship scenarios

**When to keep disabled:**
- Application is for individual users only
- No delegation scenarios exist
- Security policy prohibits acting on behalf of others

**UI Location:** Toggle at top of Stewardship Settings page

---

### Stewardship Relation Types

**Property:** `stewardship_relation_types`
**Type:** Array of Strings
**Default:** `["PARENT_OF", "GUARDIAN_OF"]`

Specifies which relationship types automatically grant stewardship when they exist between users.

**Example configurations:**

**Family/household app:**
```yaml
stewardship_relation_types:
  - "PARENT_OF"
  - "GUARDIAN_OF"
```

**Corporate app with organizational hierarchy:**
```yaml
stewardship_relation_types:
  - "MANAGER_OF"
  - "HAS_ASSISTANT"
```

**Healthcare app:**
```yaml
stewardship_relation_types:
  - "GUARDIAN_OF"
  - "CARE_COORDINATOR_OF"  # custom relation type
```

**Note:** These relation types must exist as RelationType nodes. See "Managing Relation Types" below.

**UI Location:** Multi-select dropdown in "Stewardship Relation Types" section

---

### Require Minor for Parent Relationship

**Property:** `require_minor_for_parent`
**Type:** Boolean
**Default:** `true`

When enabled, PARENT_OF relationships only grant stewardship if the ward is below the minor age threshold. When disabled, PARENT_OF always grants stewardship regardless of age.

**Recommended settings:**
- **Enable** for family apps where adult children should be independent
- **Disable** if parents should always have some access to adult children's accounts

**Example scenario:**
```
With require_minor_for_parent: true
  John --[PARENT_OF]--> Emma (age 12) = Stewardship granted
  John --[PARENT_OF]--> Emma (age 25) = No stewardship

With require_minor_for_parent: false
  John --[PARENT_OF]--> Emma (age 12) = Stewardship granted
  John --[PARENT_OF]--> Emma (age 25) = Stewardship granted
```

**UI Location:** Checkbox in "Parent Relationship Settings" section

---

### Allowed Workflows

**Property:** `allowed_workflows`
**Type:** Array of Strings
**Default:** `["invitation", "admin_assignment", "steward_creates_ward"]`

Controls which workflows can be used to establish stewardship relationships.

**Options:**

1. **`invitation`** - Ward invites steward or steward requests stewardship
   - User-initiated
   - Requires ward consent
   - Good for: Adult users choosing care coordinators, patients selecting helpers

2. **`admin_assignment`** - Administrator directly creates relationship
   - Admin-initiated
   - Can bypass ward consent
   - Good for: Verified legal guardianships, employment relationships

3. **`steward_creates_ward`** - Steward creates new ward account
   - Steward-initiated
   - No ward consent needed (ward doesn't exist yet)
   - Good for: Parents creating children's accounts, HR onboarding employees
   - Requires `steward_creates_ward_enabled: true`

**Example configurations:**

**High-security healthcare:**
```yaml
allowed_workflows:
  - "invitation"  # Patients must consent
  - "admin_assignment"  # Legal guardianships verified by admin
```

**Family application:**
```yaml
allowed_workflows:
  - "steward_creates_ward"  # Parents create children
  - "admin_assignment"  # Customer support can help
```

**Corporate environment:**
```yaml
allowed_workflows:
  - "admin_assignment"  # HR controls all delegations
```

**UI Location:** Checkboxes in "Allowed Workflows" section

---

### Steward Creates Ward Enabled

**Property:** `steward_creates_ward_enabled`
**Type:** Boolean
**Default:** `false`

Enables the "steward creates ward" workflow where stewards can create new user accounts that automatically become their wards.

**Security considerations:**
- Allows users to create accounts for others
- New accounts bypass normal registration
- May violate email verification policies
- Should be carefully controlled

**When to enable:**
- Parents should be able to create children's accounts
- HR needs to onboard employees on behalf of managers
- Guardian registration scenarios

**When to disable:**
- All users must register themselves
- Email verification is mandatory
- Security policy requires individual account creation

**Note:** Must also include `"steward_creates_ward"` in `allowed_workflows`.

**UI Location:** Toggle in "Ward Creation Settings" section

---

### Maximum Stewards Per Ward

**Property:** `max_stewards_per_ward`
**Type:** Integer
**Default:** `5`

Maximum number of stewards a single ward can have.

**Considerations:**
- Prevents excessive access to a single account
- Balance between flexibility and security
- Consider use case requirements

**Recommended values:**
- **Family apps:** 5-10 (multiple parents, grandparents, etc.)
- **Healthcare:** 2-3 (primary care coordinator plus backup)
- **Corporate:** 1-2 (assistant plus backup)

**UI Location:** Number input in "Limits" section

---

### Maximum Wards Per Steward

**Property:** `max_wards_per_steward`
**Type:** Integer
**Default:** `10`

Maximum number of wards a single steward can manage.

**Considerations:**
- Prevents steward account abuse
- Reflects realistic management capacity
- Consider typical family sizes or organizational spans

**Recommended values:**
- **Family apps:** 10-15 (large families)
- **Healthcare:** 20-50 (care coordinator caseload)
- **Corporate:** 5-10 (assistant covering multiple executives)

**UI Location:** Number input in "Limits" section

---

### Invitation Expiry Days

**Property:** `invitation_expiry_days`
**Type:** Integer
**Default:** `7`

Number of days until a stewardship invitation expires if not accepted.

**Considerations:**
- Shorter = more secure (invitations don't linger)
- Longer = more convenient (users have time to respond)
- Consider user technical proficiency

**Recommended values:**
- **General purpose:** 7 days
- **High security:** 3 days
- **Elderly/non-technical users:** 14-30 days

**UI Location:** Number input in "Invitation Settings" section

---

### Require Ward Consent

**Property:** `require_ward_consent`
**Type:** Boolean
**Default:** `true`

Whether ward must consent to stewardship relationships.

**When enabled:**
- Ward receives invitation/request
- Ward must explicitly accept
- More privacy-protective

**When disabled:**
- Relationships can be established without ward action
- Useful for minors who cannot meaningfully consent
- Required for admin assignment of legal guardianships

**Important:** This setting interacts with minor status:
- If ward is minor AND `allow_minor_login: false`, consent cannot be obtained
- In such cases, steward or admin must establish relationship

**UI Location:** Toggle in "Consent Settings" section

---

### Minor Age Threshold

**Property:** `minor_age_threshold`
**Type:** Integer
**Default:** `18`

Age below which a user is considered a minor.

**Effects of minor status:**
- Determines if PARENT_OF grants stewardship (if `require_minor_for_parent: true`)
- May affect consent requirements
- May determine login permissions

**Recommended values:**
- **18:** Standard age of majority in many jurisdictions
- **13:** COPPA compliance in US (Children's Online Privacy Protection Act)
- **16:** Some European jurisdictions
- **21:** Certain regulated industries

**Important:** Check your jurisdiction's legal requirements.

**UI Location:** Number input in "Age Settings" section

---

### Allow Minor Login

**Property:** `allow_minor_login`
**Type:** Boolean
**Default:** `false`

Whether users below the minor age threshold can log in directly to the system.

**When disabled (default):**
- Minors cannot log in
- All access must be through stewards
- Maximum privacy protection for children

**When enabled:**
- Minors can log in with their own credentials
- Minors can view actions stewards take on their behalf
- Minors can still be managed by stewards
- Appropriate for older children (13-17)

**Use cases for enabling:**
- Teenagers managing their own accounts with parent oversight
- Educational applications where students need direct access
- Apps where minors are primary users

**Use cases for disabling:**
- Young children's accounts
- Maximum privacy protection
- Parents want full control

**UI Location:** Toggle in "Minor Access Settings" section

---

## Managing Relation Types

Relation types define the types of relationships that can exist between users and determine which relationships grant stewardship.

### Viewing Existing Relation Types

**In Admin Console:**
1. Navigate to "Access Control" → "Relation Types"
2. You'll see a list of all configured relation types
3. Each shows:
   - Relation name (e.g., "PARENT_OF")
   - Display title
   - Category
   - Whether it implies stewardship
   - Icon and color

### Creating a New Relation Type

**Example: Adding a "CARE_COORDINATOR_OF" relation for a healthcare app**

1. **Navigate to Relation Types Manager**
   - Access Control → Relation Types
   - Click "Create New Relation Type"

2. **Fill in Basic Information**
   - **Relation Name:** `CARE_COORDINATOR_OF` (UPPER_SNAKE_CASE)
   - **Title:** `Care Coordinator Of` (display name for UI)
   - **Description:** `Professional care coordinator assigned to patient`
   - **Category:** `healthcare` (for grouping in UI)

3. **Configure Relationship Behavior**
   - **Inverse Relation Name:** `HAS_CARE_COORDINATOR` (optional)
   - **Bidirectional:** No (unchecked)
     - If checked, relationship exists both ways automatically
   - **Implies Stewardship:** Yes (checked)
     - This relation type will grant stewardship
   - **Requires Minor:** No (unchecked)
     - Stewardship granted regardless of age

4. **Visual Settings**
   - **Icon:** `heart` (choose from icon library)
   - **Color:** `#ef4444` (hex color for UI display)

5. **Save**
   - Click "Create Relation Type"
   - New relation type is immediately available

6. **Add to Stewardship Configuration**
   - Return to Stewardship Settings
   - Add `CARE_COORDINATOR_OF` to `stewardship_relation_types` list
   - Save

### Modifying Existing Relation Types

1. Navigate to Relation Types list
2. Click on the relation type to edit
3. Modify properties as needed
4. Save changes

**Warning:** Changing `implies_stewardship` affects existing relationships:
- Enabling it grants stewardship to existing relationships
- Disabling it revokes stewardship from existing relationships

### Default Relation Types

The stewardship package includes these relation types out of the box:

#### Household Relationships
| Relation | Inverse | Implies Stewardship | Requires Minor |
|----------|---------|-------------------|----------------|
| PARENT_OF | CHILD_OF | Yes | Yes |
| GUARDIAN_OF | WARD_OF | Yes | No |
| SPOUSE_OF | SPOUSE_OF | No | No |
| SIBLING_OF | SIBLING_OF | No | No |
| GRANDPARENT_OF | GRANDCHILD_OF | No | No |

#### Organization Relationships
| Relation | Inverse | Implies Stewardship | Requires Minor |
|----------|---------|-------------------|----------------|
| MANAGER_OF | REPORTS_TO | No* | No |
| HAS_ASSISTANT | ASSISTANT_OF | No* | No |

\* *Can be configured to imply stewardship if needed*

---

## Managing Entity Circles

Entity Circles group users into families, teams, or organizational units.

### Viewing Entity Circles

**In Admin Console:**
1. Navigate to "Access Control" → "Entity Circles"
2. See list of all circles with members
3. Filter by circle type (family, team, org_unit, etc.)

### Creating an Entity Circle

**Example: Creating the "Smith Family" circle**

1. **Navigate to Entity Circles**
   - Access Control → Entity Circles
   - Click "Create New Circle"

2. **Fill in Information**
   - **Name:** `Smith Family`
   - **Circle Type:** `family` (or choose: team, org_unit, custom)
   - **Primary Contact:** Select John Smith (user dropdown)
   - **Address:** (optional)
     ```json
     {
       "street": "123 Main St",
       "city": "Springfield",
       "state": "IL",
       "zip": "62701",
       "country": "USA"
     }
     ```
   - **Metadata:** (optional, extensible JSON)
     ```json
     {
       "household_size": 4,
       "emergency_contact": "+1-555-0123"
     }
     ```

3. **Add Members**
   - Click "Add Member"
   - Search and select: John Smith, Maria Smith, Emma Smith, Oliver Smith
   - Save

### Managing Circle Membership

1. Open the Entity Circle
2. **Add members:**
   - Click "Add Member"
   - Select user from dropdown
   - Save
3. **Remove members:**
   - Click X next to member name
   - Confirm removal

### Circle Types

Common circle types and their uses:

- **`family`**: Household units
  - Use for: Family management, shared household resources
  - Typical size: 2-10 members

- **`team`**: Work teams or project groups
  - Use for: Project collaboration, team resources
  - Typical size: 5-15 members

- **`org_unit`**: Organizational departments
  - Use for: Department-level permissions, reporting structure
  - Typical size: 10-100 members

- **Custom types**: Define your own as needed

---

## Common Configuration Scenarios

### Scenario 1: Family/Household Application

**Goal:** Parents manage children's accounts until age 18

```yaml
# Stewardship Settings
enabled: true
stewardship_relation_types:
  - "PARENT_OF"
  - "GUARDIAN_OF"
require_minor_for_parent: true
allowed_workflows:
  - "invitation"
  - "admin_assignment"
  - "steward_creates_ward"
steward_creates_ward_enabled: true
max_stewards_per_ward: 5
max_wards_per_steward: 10
invitation_expiry_days: 7
require_ward_consent: false
minor_age_threshold: 18
allow_minor_login: false
```

**Steps:**
1. Enable stewardship
2. Keep default PARENT_OF and GUARDIAN_OF relation types
3. Enable all workflows for flexibility
4. Set minor threshold to 18
5. Disable minor login for young children
6. Disable ward consent (minors can't consent)

---

### Scenario 2: Corporate Application with Assistants

**Goal:** Executives delegate specific tasks to assistants

```yaml
# Stewardship Settings
enabled: true
stewardship_relation_types:
  - "HAS_ASSISTANT"
require_minor_for_parent: true  # N/A for this use case
allowed_workflows:
  - "admin_assignment"
steward_creates_ward_enabled: false
max_stewards_per_ward: 2
max_wards_per_steward: 3
invitation_expiry_days: 7
require_ward_consent: true
minor_age_threshold: 18
allow_minor_login: true  # N/A for this use case
```

**Additional steps:**
1. Enable stewardship
2. Add "HAS_ASSISTANT" to stewardship relation types
3. Modify HAS_ASSISTANT relation type to set `implies_stewardship: true`
4. Only allow admin assignment workflow (HR controls)
5. Limit to 2 stewards per executive (primary + backup)
6. Limit assistants to 3 executives each
7. Use StewardshipOverrides for scoped permissions (see below)

---

### Scenario 3: Healthcare Application

**Goal:** Care coordinators help elderly patients with consent

```yaml
# Stewardship Settings
enabled: true
stewardship_relation_types:
  - "GUARDIAN_OF"
  - "CARE_COORDINATOR_OF"  # Custom relation type
require_minor_for_parent: true
allowed_workflows:
  - "invitation"
  - "admin_assignment"
steward_creates_ward_enabled: false
max_stewards_per_ward: 3
max_wards_per_steward: 30
invitation_expiry_days: 14
require_ward_consent: true
minor_age_threshold: 18
allow_minor_login: true
```

**Additional steps:**
1. Create custom "CARE_COORDINATOR_OF" relation type
2. Set `implies_stewardship: true` on the relation type
3. Add to stewardship_relation_types list
4. Extend invitation expiry for non-technical users
5. Allow higher ward count per care coordinator (caseload)
6. Require patient consent for privacy compliance

---

## Creating Scoped Delegations

For fine-grained control, use StewardshipOverride records instead of relation-based stewardship.

### Via Admin Console

1. **Navigate to Stewardship Overrides**
   - Access Control → Stewardship Overrides
   - Click "Create New Override"

2. **Select Steward and Ward**
   - **Steward:** Search and select user (e.g., Sarah Johnson)
   - **Ward:** Search and select user (e.g., Michael Chen)

3. **Configure Delegation**
   - **Delegation Mode:** Select "Scoped" or "Full"

   **For Scoped:**
   - **Scoped Permissions:** Define specific permissions
     ```json
     [
       {
         "resource": "calendar",
         "actions": ["read", "write"]
       },
       {
         "resource": "meetings",
         "actions": ["read", "approve"]
       },
       {
         "resource": "travel",
         "actions": ["read", "write"]
       }
     ]
     ```

4. **Set Time Bounds** (optional)
   - **Valid From:** Select start date (or leave blank for immediate)
   - **Valid Until:** Select end date (or leave blank for indefinite)

5. **Add Context**
   - **Reason:** `Parental leave coverage` (optional but recommended)
   - **Status:** Leave as "pending" (becomes "active" automatically)

6. **Save**
   - Click "Create Override"
   - Notifications sent to both users

### Permission Structure

Scoped permissions are defined as arrays of permission objects:

```json
[
  {
    "resource": "resource_type",
    "actions": ["action1", "action2"],
    "conditions": {
      "optional_constraint": "value"
    }
  }
]
```

**Example: Assistant with calendar and email access**
```json
[
  {
    "resource": "calendar",
    "actions": ["read", "write", "delete"]
  },
  {
    "resource": "email",
    "actions": ["read", "send"],
    "conditions": {
      "folder": "inbox"
    }
  },
  {
    "resource": "documents",
    "actions": ["read"],
    "conditions": {
      "classification": ["public", "internal"]
    }
  }
]
```

---

## Monitoring and Auditing

### Viewing Stewardship Activity

**In Admin Console:**

1. **Navigate to Audit Logs**
   - Access Control → Audit Logs
   - Filter by "Stewardship Actions"

2. **View Log Entries**
   Each entry shows:
   - **Action:** What was done
   - **Performed By:** Steward user
   - **On Behalf Of:** Ward user
   - **Timestamp:** When it occurred
   - **Details:** Additional context

3. **Filter Options**
   - By steward user
   - By ward user
   - By date range
   - By action type

### Stewardship Reports

Access pre-built reports:

1. **Active Stewardships Report**
   - Shows all active steward-ward relationships
   - Grouped by steward or ward
   - Export to CSV

2. **Expiring Delegations Report**
   - StewardshipOverrides expiring soon
   - Allows proactive renewal or transition

3. **Stewardship Activity Report**
   - Volume of steward actions over time
   - Breakdown by action type
   - Useful for compliance and oversight

---

## Troubleshooting

### Issue: Stewardship Not Being Granted

**Possible causes:**

1. **Stewardship disabled**
   - Check: `enabled: true` in StewardshipConfig

2. **Relation type not in stewardship list**
   - Check: Relation type is in `stewardship_relation_types` array
   - Verify: Relation type has `implies_stewardship: true`

3. **Minor requirement not met**
   - Check: If using PARENT_OF with `require_minor_for_parent: true`
   - Verify: Ward's age is below `minor_age_threshold`

4. **Workflow not allowed**
   - Check: The workflow being used is in `allowed_workflows`

5. **Limits exceeded**
   - Check: Ward doesn't have more than `max_stewards_per_ward`
   - Check: Steward doesn't have more than `max_wards_per_steward`

### Issue: Ward Cannot Consent to Stewardship

**Possible causes:**

1. **Minor without login access**
   - If ward is minor and `allow_minor_login: false`
   - Solution: Use admin assignment or steward-initiated workflow

2. **Account not activated**
   - Ward hasn't completed registration
   - Solution: Ensure ward activates account first

### Issue: Scoped Permissions Not Working

**Possible causes:**

1. **Permission structure incorrect**
   - Verify JSON format in scoped_permissions
   - Check for syntax errors

2. **Resource types don't match application**
   - Ensure resource names match your application's resources
   - Consult developer documentation

3. **Override status not active**
   - Check status is "active", not "pending" or "expired"
   - Check date range if specified

---

## Security Best Practices

### 1. Principle of Least Privilege
- Use scoped delegations instead of full delegations when possible
- Only grant necessary permissions
- Regularly review and revoke unnecessary stewardships

### 2. Time-Bound Delegations
- Set expiration dates for temporary access
- Review expiring delegations regularly
- Don't use indefinite delegations unless truly needed

### 3. Audit Regularly
- Review stewardship activity logs monthly
- Investigate unusual patterns
- Ensure stewardships align with organizational policies

### 4. Require Consent When Appropriate
- Enable `require_ward_consent: true` for adult users
- Document cases where consent is bypassed
- Ensure legal basis for non-consensual stewardships

### 5. Limit Steward Counts
- Set reasonable `max_stewards_per_ward` and `max_wards_per_steward`
- Review outliers (users with many stewards or wards)
- Investigate before raising limits

### 6. Document Stewardship Policies
- Create written policies for stewardship use
- Train administrators on proper configuration
- Communicate stewardship capabilities to users

---

## API Reference

For programmatic configuration and management, see the developer API documentation:

- **Configuration API:** `/docs/developer/stewardship/api.md`
- **Stewardship Resolution:** `/docs/developer/stewardship/resolution.md`
- **Event Hooks:** `/docs/developer/stewardship/events.md`

---

## Next Steps

- **Understand the concepts:** See [Overview](./overview.md)
- **Review use cases:** See [Use Cases](./use-cases.md)
- **Reference terminology:** See [Glossary](./glossary.md)

## Support

For assistance with configuration:
- Consult the [Glossary](./glossary.md) for terminology
- Review [Use Cases](./use-cases.md) for scenario-based guidance
- Contact RaisinDB support for complex configurations
