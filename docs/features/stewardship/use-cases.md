# Stewardship Use Cases

This guide walks through common scenarios where the stewardship system provides value, with detailed step-by-step flows for each.

## Use Case 1: Household/Family Management

### Scenario: Parents Managing Children's Accounts

The Smith family uses your application for managing school activities, medical appointments, and extracurricular programs. The parents need to manage their children's accounts until they reach adulthood.

#### Actors
- **John Smith** (Father, age 42)
- **Maria Smith** (Mother, age 40)
- **Emma Smith** (Daughter, age 12)
- **Oliver Smith** (Son, age 8)

### Flow 1A: Father Registers and Creates Children as Wards

**Initial Setup (Steward Creates Ward workflow)**

1. **John registers for an account**
   - John visits the application and creates his account
   - Provides his information including date of birth (making him an adult user)
   - Completes registration and logs in

2. **John creates Emma's account**
   - John navigates to "Family Management" or "Add Ward"
   - Selects "Create child account"
   - Enters Emma's information:
     - Full name: Emma Smith
     - Date of birth: (12 years ago)
     - Email: emma.smith@example.com (optional, for future use)
   - Specifies relationship type: PARENT_OF
   - System automatically:
     - Creates Emma's user account
     - Establishes PARENT_OF relationship from John to Emma
     - Grants John stewardship over Emma (because Emma is a minor)
     - Creates or updates the "Smith Family" EntityCircle

3. **John creates Oliver's account**
   - Repeats the same process for Oliver
   - System establishes John as steward for Oliver

**Result:**
```
John --[PARENT_OF]--> Emma (stewardship: full)
John --[PARENT_OF]--> Oliver (stewardship: full)
```

### Flow 1B: Mother Joins and Becomes Co-Parent

**Adding a Second Steward**

1. **Maria registers for an account**
   - Maria creates her own account independently
   - Provides her information

2. **John invites Maria to join the family**
   - Option A: John adds Maria to the "Smith Family" EntityCircle
   - Option B: John creates a SPOUSE_OF relationship with Maria

3. **Establishing Maria as steward for the children**

   **Method 1: Invitation workflow**
   - Emma (through John acting as steward) sends invitation to Maria
   - System sends notification to Maria
   - Maria reviews and accepts invitation
   - PARENT_OF relationship created: Maria → Emma
   - Stewardship automatically granted

   **Method 2: John acts as steward**
   - John, acting on behalf of Emma, establishes the PARENT_OF relationship to Maria
   - Since John is Emma's steward, he can manage her relationships
   - System creates the relationship and grants Maria stewardship

   **Method 3: Admin assignment**
   - Family administrator directly creates the relationships
   - Both Maria and Emma receive notifications

4. **Repeat for Oliver**
   - Establish Maria as steward for Oliver using same method

**Result:**
```
John --[PARENT_OF]--> Emma
Maria --[PARENT_OF]--> Emma
John --[PARENT_OF]--> Oliver
Maria --[PARENT_OF]--> Oliver

EntityCircle: "Smith Family"
Members: John, Maria, Emma, Oliver
```

### Flow 1C: Managing School Registration

**Using Stewardship for Day-to-Day Tasks**

1. **School registration opens**
   - The school sends notification about open enrollment
   - Email goes to Emma's account but parents can see it

2. **Maria registers Emma for classes**
   - Maria logs into the application
   - Switches context to "Acting as Emma" (steward mode)
   - Navigates to school registration
   - Fills out forms and selects classes
   - Submits registration
   - System records:
     - Action: "Register for classes"
     - Performed by: Maria Smith (steward)
     - On behalf of: Emma Smith (ward)
     - Timestamp and details

3. **Audit trail shows both identities**
   - Emma's activity log shows: "Registered for Math 101, Science 102 (by Maria Smith)"
   - Maria's activity log shows: "Registered Emma Smith for Math 101, Science 102"
   - School administrators see registration from Emma's account with steward notation

### Flow 1D: Child Reaching Adulthood

**Automatic Stewardship Transition**

1. **Emma turns 18**
   - System detects Emma's birthday based on date of birth
   - Emma's minor status automatically changes to false

2. **System evaluates stewardship relationships**
   - PARENT_OF relationships from John and Maria still exist
   - However, since PARENT_OF is configured with `requires_minor: true`
   - Stewardship is automatically revoked

3. **Notification and transition**
   - Emma receives notification: "You now have full control of your account"
   - John and Maria receive notification: "Stewardship for Emma has ended (age 18 reached)"
   - Emma can now manage her account independently
   - Historical records of steward actions remain in audit log

4. **Optional: Adult-to-adult relationship**
   - The PARENT_OF relationship can remain for other purposes (family tree, etc.)
   - If desired, Emma can grant specific limited access to parents through scoped delegation

**Result:**
```
John --[PARENT_OF]--> Emma (stewardship: none - Emma is adult)
Maria --[PARENT_OF]--> Emma (stewardship: none - Emma is adult)
```

---

## Use Case 2: Organization Delegation

### Scenario: Executive Assistant Managing Tasks

Sarah is the executive assistant to Michael, the VP of Sales. She needs to manage his calendar, approve certain requests, and handle scheduling, but should not have access to confidential financial information.

#### Actors
- **Michael Chen** (VP of Sales)
- **Sarah Johnson** (Executive Assistant)

### Flow 2A: Establishing Scoped Delegation

1. **Admin creates the delegation**
   - System administrator or Michael's manager navigates to stewardship management
   - Creates new StewardshipOverride:
     - Steward: Sarah Johnson
     - Ward: Michael Chen
     - Delegation mode: Scoped
     - Scoped permissions:
       - Can view and edit calendar
       - Can approve meeting requests
       - Can manage travel arrangements
       - Cannot access financial records
       - Cannot approve contracts over $10,000
     - Valid from: (today)
     - Valid until: (null - indefinite until revoked)

2. **Optional: Create relationship for organizational chart**
   - Create HAS_ASSISTANT relationship: Michael → Sarah
   - This relationship shows in org chart but doesn't automatically grant stewardship
   - Stewardship comes from the StewardshipOverride

3. **Notifications sent**
   - Sarah receives: "You have been granted assistant access to Michael Chen's account"
   - Michael receives: "Sarah Johnson now has assistant access to your account"
   - Details of permissions included in notifications

### Flow 2B: Sarah Managing Michael's Calendar

1. **Calendar invitation arrives**
   - Client requests meeting with Michael
   - Invitation shows in Michael's calendar

2. **Sarah reviews and responds**
   - Sarah logs into the application
   - Switches to "Acting as Michael Chen" (steward mode)
   - Reviews Michael's calendar and availability
   - Accepts meeting invitation
   - System checks:
     - Is Sarah a steward of Michael? Yes
     - Is this action within scoped permissions? Yes (calendar management)
     - Action allowed

3. **Action recorded**
   - Calendar shows: "Meeting accepted by Sarah Johnson on behalf of Michael Chen"
   - Both Michael and Sarah can see the booking
   - Audit log records both identities

### Flow 2C: Permission Boundary Enforcement

1. **Sarah attempts to access financial records**
   - Sarah (acting as Michael) navigates to financial reports
   - System checks:
     - Is Sarah a steward of Michael? Yes
     - Is this action within scoped permissions? No (financial access not granted)
     - Action denied

2. **User feedback**
   - Sarah sees: "You do not have permission to access financial records for Michael Chen"
   - Option to request additional permissions
   - Security log records the attempted access (for audit purposes)

---

## Use Case 3: Temporary Access for Vacation Coverage

### Scenario: Manager Away on Extended Leave

Alex is going on parental leave for 3 months. Jordan will cover some of Alex's responsibilities during this time, but only needs temporary access.

#### Actors
- **Alex Rivera** (Engineering Manager, on leave)
- **Jordan Taylor** (Senior Engineer, covering)

### Flow 3A: Setting Up Time-Limited Delegation

1. **Before leave begins**
   - Alex's manager creates StewardshipOverride:
     - Steward: Jordan Taylor
     - Ward: Alex Rivera
     - Delegation mode: Scoped
     - Scoped permissions:
       - Can approve time-off requests for team
       - Can review and comment on code reviews
       - Can update project status
       - Cannot approve hiring decisions
       - Cannot change team member compensation
     - Valid from: 2025-03-01 (leave start date)
     - Valid until: 2025-06-01 (leave end date)
     - Reason: "Parental leave coverage"
     - Status: pending (becomes active on start date)

2. **Notifications**
   - Alex receives: "Jordan Taylor will have temporary coverage access during your leave"
   - Jordan receives: "You will have temporary access to Alex Rivera's account from March 1 to June 1"

### Flow 3B: During Leave Period

1. **Coverage begins**
   - On March 1, status automatically changes to "active"
   - Jordan can now act as Alex within scoped permissions

2. **Jordan approves time-off request**
   - Team member submits vacation request
   - Jordan (acting as Alex) reviews and approves
   - System records: "Approved by Jordan Taylor on behalf of Alex Rivera"

3. **Jordan monitors activity**
   - Can view dashboards and reports
   - Can update project statuses
   - Cannot make permanent organizational changes

### Flow 3C: Automatic Expiration

1. **Leave ends**
   - On June 1, system automatically:
     - Changes StewardshipOverride status to "expired"
     - Revokes Jordan's stewardship access
     - Sends notifications to both Alex and Jordan

2. **Jordan loses access**
   - Jordan can no longer act as Alex
   - All actions during the coverage period remain in audit log
   - Alex can review all actions taken on their behalf during leave

3. **Handoff**
   - Alex reviews activity log upon return
   - Can see all decisions made by Jordan
   - Can follow up on any items as needed

---

## Use Case 4: Legal Guardianship

### Scenario: Non-Parent Guardian for a Minor

Lisa is the legal guardian for her nephew Ryan (age 14) after a family situation. Ryan's parents are not involved, and Lisa needs full stewardship to manage his account.

#### Actors
- **Lisa Anderson** (Aunt and legal guardian, age 45)
- **Ryan Martinez** (Nephew, age 14)

### Flow 4A: Establishing Guardianship

**Option 1: Admin assignment (recommended for legal guardianship)**

1. **Lisa provides legal documentation**
   - Lisa contacts application support
   - Provides legal guardianship documentation
   - Support creates support ticket for verification

2. **Admin verifies and creates stewardship**
   - Support team verifies legal documentation
   - Admin creates relationship in system:
     - Relationship type: GUARDIAN_OF
     - From: Lisa Anderson
     - To: Ryan Martinez
   - Since GUARDIAN_OF has `implies_stewardship: true`
   - System automatically grants Lisa stewardship over Ryan

3. **EntityCircle created**
   - Admin creates "Anderson-Martinez Family" EntityCircle
   - Members: Lisa, Ryan

**Option 2: Invitation workflow (if Ryan has existing account)**

1. **Ryan's existing account**
   - Ryan already has an account from before
   - Account is marked as minor (age 14)

2. **Lisa sends guardianship request**
   - Lisa creates account
   - Navigates to "Establish guardianship"
   - Enters Ryan's information
   - Sends guardianship invitation

3. **Admin approval required**
   - Since this is a GUARDIAN_OF relationship (serious legal implication)
   - System requires admin verification
   - Lisa uploads legal documentation
   - Admin reviews and approves
   - Relationship created and stewardship granted

### Flow 4B: Full Account Management

1. **Lisa manages all aspects of Ryan's account**
   - Can update Ryan's profile information
   - Can manage privacy settings
   - Can review all activity
   - Can make purchases on Ryan's behalf
   - Can communicate with service providers

2. **Ryan's perspective**
   - Ryan can still log in (if `allow_minor_login: true`)
   - Can see that Lisa is his guardian
   - Can view actions Lisa takes on his behalf
   - Cannot revoke the guardianship (must be done by admin or when reaching adulthood)

### Flow 4C: Transitioning to Adulthood

1. **Ryan approaches age 18**
   - System sends notification 30 days before birthday
   - Notifies both Lisa and Ryan of upcoming transition

2. **Ryan turns 18**
   - Minor status changes to false
   - Since GUARDIAN_OF has `requires_minor: false` (guardianship can persist)
   - Stewardship remains unless explicitly configured otherwise

3. **Ryan chooses next steps**
   - Option A: Revoke guardianship stewardship, become fully independent
   - Option B: Convert to scoped delegation (Lisa helps with specific tasks)
   - Option C: Maintain guardianship relationship (if Ryan needs continued support)
   - Ryan now has full control to make this decision

---

## Use Case 5: Healthcare Provider Scenario

### Scenario: Care Coordinator Managing Patient Accounts

A healthcare application uses stewardship for care coordinators to manage elderly or disabled patients' accounts.

#### Actors
- **Patricia Wu** (Care coordinator)
- **George Thompson** (Patient, age 78, limited mobility)

### Flow 5A: Patient Consent and Delegation

1. **George registers with healthcare provider**
   - Creates account during intake process
   - Expresses desire for help managing appointments

2. **Care coordinator assignment**
   - Healthcare provider assigns Patricia as George's care coordinator
   - Admin creates StewardshipOverride:
     - Steward: Patricia Wu
     - Ward: George Thompson
     - Delegation mode: Scoped
     - Scoped permissions:
       - Schedule and manage appointments
       - View medical history and care plans
       - Communicate with healthcare providers
       - Cannot access billing information (George retains control)
       - Cannot change emergency contacts
     - Valid from: (assignment date)
     - Valid until: null (ongoing)

3. **Patient consent**
   - George receives notification
   - Reviews permissions
   - Provides explicit consent through system
   - Status changes to "active" after consent

### Flow 5B: Care Coordination Activities

1. **Patricia schedules appointment**
   - Receives notification that George needs follow-up
   - Logs in and acts as George
   - Schedules appointment at convenient time
   - George receives confirmation

2. **Patricia coordinates care**
   - Reviews George's care plan
   - Communicates with specialists
   - Updates care notes
   - All actions recorded with both identities

### Flow 5C: Revocation by Patient

1. **George decides to manage independently**
   - George's mobility improves
   - Decides to manage own appointments
   - Logs into account
   - Navigates to "Manage stewards"
   - Revokes Patricia's stewardship

2. **Immediate effect**
   - Patricia loses access to act as George
   - Patricia receives notification
   - Care coordinator supervisor is notified
   - Historical actions remain in audit log

---

## Comparison Table: Workflow Selection

| Workflow | When to Use | Ward Consent | Examples |
|----------|-------------|--------------|----------|
| **Steward Creates Ward** | New accounts being created by steward | Not required (ward doesn't exist yet) | Parent creating child account, HR creating employee accounts |
| **Invitation** | Existing accounts, ward-initiated | Required (ward sends invitation) | Adult choosing to add guardian, patient selecting care coordinator |
| **Admin Assignment** | Legal requirements, verified relationships | Configurable (may bypass for minors) | Court-ordered guardianship, verified employment relationships |

## Configuration Recommendations by Use Case

### Family/Household
```yaml
enabled: true
stewardship_relation_types: ["PARENT_OF", "GUARDIAN_OF"]
require_minor_for_parent: true
allowed_workflows: ["invitation", "admin_assignment", "steward_creates_ward"]
steward_creates_ward_enabled: true
minor_age_threshold: 18
allow_minor_login: false
require_ward_consent: false  # minors may not be able to consent
```

### Organization
```yaml
enabled: true
stewardship_relation_types: ["MANAGER_OF"]  # optional, may prefer overrides only
allowed_workflows: ["admin_assignment"]
require_ward_consent: true
max_stewards_per_ward: 2
max_wards_per_steward: 20
```

### Healthcare
```yaml
enabled: true
stewardship_relation_types: ["GUARDIAN_OF"]
allowed_workflows: ["invitation", "admin_assignment"]
require_ward_consent: true
invitation_expiry_days: 30
```

---

## Next Steps

- **Configure stewardship for your use case**: See [Configuration Guide](./configuration.md)
- **Understand the concepts**: See [Overview](./overview.md)
- **Reference terminology**: See [Glossary](./glossary.md)
