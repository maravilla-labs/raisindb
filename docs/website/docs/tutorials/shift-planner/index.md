# Shift Planner: Employee Scheduling System

Build an employee shift scheduling system with approval workflows, constraint validation, and branch-based planning.

:::info What You'll Learn
- Model complex relationships (employees, shifts, preferences)
- Use branches for draft schedules and approval workflows
- Query graph-like data with RaisinSQL
- Implement constraint-based validation patterns
:::

## Prerequisites

- Completed the [Quickstart Tutorial](/docs/tutorials/quickstart)
- RaisinDB running locally

## What We're Building

A shift scheduling system that:
1. Manages employees and their availability preferences
2. Creates shift schedules as draft branches
3. Validates constraints (overtime limits, required rest periods)
4. Supports manager approval before publishing to main

---

## Step 1: Model the Domain

<!--
OUTLINE FOR AUTHOR:

Create four NodeTypes:

**Employee NodeType:**
- employee_id (string, unique)
- name (string)
- role (enum: nurse, doctor, technician, manager)
- max_hours_per_week (integer, default 40)
- email (string, optional - for notifications)

**Preference NodeType:**
- employee_id (reference)
- day_of_week (enum: mon, tue, wed, thu, fri, sat, sun)
- preferred_shift (enum: morning, afternoon, night, off)
- priority (enum: hard, soft) - hard = must respect, soft = try to respect

**Shift NodeType:**
- shift_id (string)
- date (date)
- shift_type (enum: morning, afternoon, night)
- start_time (time)
- end_time (time)
- required_staff (object: {nurse: 2, doctor: 1})

**Assignment NodeType:**
- shift_id (reference)
- employee_id (reference)
- assigned_role (string)
- status (enum: scheduled, confirmed, swapped, cancelled)

Tips:
- Explain the relationship model
- Show how to query across relationships
-->

```bash
# TODO: Add NodeType creation commands
```

---

## Step 2: Seed Employee and Preference Data

<!--
OUTLINE FOR AUTHOR:
- Create 6-8 employees with different roles
- Add preferences for each (some want mornings, some can't work weekends)
- Include one employee with overtime restrictions
-->

```bash
# TODO: Add employee and preference creation
```

---

## Step 3: Create Shifts for the Week

<!--
OUTLINE FOR AUTHOR:
- Create a week's worth of shifts
- Show the staffing requirements per shift
- Calculate total hours available vs needed
-->

```bash
# TODO: Add shift creation commands
```

---

## Step 4: Draft Schedule on a Branch

<!--
OUTLINE FOR AUTHOR:

This is the key workflow:

1. Create branch "schedule-week-51" from main
2. On the branch, create Assignment documents
3. This is the "draft" - not visible on main yet
4. Manager can review, employee can see draft

Show:
- Branch creation
- Adding assignments on branch
- Querying the branch vs main (main has no assignments yet)
-->

```bash
# TODO: Add branch-based scheduling
```

---

## Step 5: Validate Constraints

<!--
OUTLINE FOR AUTHOR:

Show queries that check for:

1. **Overtime check:**
   - Query: Sum hours per employee this week
   - Flag anyone over max_hours_per_week

2. **Rest period check:**
   - Query: Find back-to-back night-then-morning shifts
   - Minimum 11 hours between shifts (EU working time directive)

3. **Preference violations:**
   - Query: Find assignments that conflict with hard preferences
   - Count soft preference violations

4. **Staffing gaps:**
   - Query: Shifts where assigned < required

Show these as SQL queries on the branch
If violations found, the schedule needs adjustment before approval
-->

```sql
-- TODO: Add constraint validation queries
```

---

## Step 6: Adjust and Re-validate

<!--
OUTLINE FOR AUTHOR:
- Show how to update assignments on the branch
- Fix one or two constraint violations
- Re-run validation queries
- When all pass, ready for approval
-->

```bash
# TODO: Add adjustment examples
```

---

## Step 7: Approval Workflow

<!--
OUTLINE FOR AUTHOR:

Two options to show:

**Option A: Merge to main**
- Manager approves → merge branch to main
- Assignments now visible on main
- Branch can be deleted

**Option B: Multi-stage approval**
- Create another branch from schedule-week-51 for employee confirmation
- Employees update their assignments to "confirmed" status
- Then merge to main

Show Option A as the simple path
Mention Option B as an extension
-->

```bash
# TODO: Add merge/approval example
```

---

## Step 8: Handle Schedule Changes

<!--
OUTLINE FOR AUTHOR:
- Employee requests shift swap
- Create a new branch from main: "swap-request-123"
- Make the change on branch
- Re-validate constraints
- Merge when approved

This shows ongoing use of branches for changes
-->

```bash
# TODO: Add swap request example
```

---

## Architecture Recap

```
main (published schedule)
  │
  ├── schedule-week-51 (draft)
  │     │
  │     └── employee-confirmations (optional sub-branch)
  │
  └── swap-request-123 (ad-hoc change)
```

**Branch Workflow:**
1. Create branch for draft
2. Make changes, validate constraints
3. Merge when approved
4. Delete branch (optional cleanup)

---

## Extending This Tutorial

Ideas for taking this further:

- **SMS/Agent Integration:** Use RaisinDB's WebSocket to trigger notifications when schedules are published
- **Preference Learning:** Track preference violations over time to improve future scheduling
- **Shift Bidding:** Let employees bid on open shifts, use branches to evaluate different bid outcomes

---

## What's Next?

You've learned how to:
- Model complex relationships for scheduling
- Use branches as draft workspaces
- Implement constraint validation with queries
- Build approval workflows with merge operations

### Continue Learning

- [Branching Concepts](/docs/why/concepts) - Deep dive into branch operations
- [Query Examples](/docs/access/sql/examples) - More complex query patterns
- [REST API Reference](/docs/access/rest/overview) - Complete API documentation

---

## Complete Code

<details>
<summary>All commands from this tutorial</summary>

```bash
# TODO: Consolidate all commands here
```

</details>
