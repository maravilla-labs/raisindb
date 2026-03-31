# Quickstart: Build a Task Manager

Build a working task management application in 15 minutes. You'll learn the core concepts of RaisinDB while creating something useful.

:::info What You'll Learn
- Define a data model with NodeTypes
- Create, read, update, and delete documents
- Query data with RaisinSQL
- Use branches for safe experimentation
:::

## Prerequisites

- RaisinDB running locally ([Installation Guide](/docs/getting-started/installation))
- curl or any REST client

---

## Step 1: Define Your Data Model

<!--
OUTLINE FOR AUTHOR:
- Explain NodeTypes briefly (link to docs for depth)
- Create a simple "Task" NodeType with:
  - title (string, required)
  - description (string, optional)
  - status (enum: todo, in_progress, done)
  - priority (integer, 1-5)
  - due_date (datetime, optional)
- Show the REST API call to create the NodeType
- Include the expected response
- Tip: Mention that NodeTypes are versioned
-->

```bash
# TODO: Add curl command to create Task NodeType
```

---

## Step 2: Create Your First Tasks

<!--
OUTLINE FOR AUTHOR:
- Create 3-4 sample tasks with varying status/priority
- Show both single create and batch create
- Explain auto-generated IDs
- Show how to retrieve the created task by ID
-->

```bash
# TODO: Add curl commands to create tasks
```

---

## Step 3: Query Your Tasks

<!--
OUTLINE FOR AUTHOR:
- Basic SELECT query
- Filter by status (WHERE status = 'todo')
- Order by priority
- Filter by due date range
- Show both REST and SQL wire protocol options
-->

```bash
# TODO: Add query examples
```

---

## Step 4: Update and Complete Tasks

<!--
OUTLINE FOR AUTHOR:
- Update a task's status
- Partial updates vs full replacement
- Show optimistic locking with revision IDs
-->

```bash
# TODO: Add update examples
```

---

## Step 5: Try Branching (Safe Experimentation)

<!--
OUTLINE FOR AUTHOR:
- This is a RaisinDB differentiator - emphasize it
- Create a branch called "experiment"
- Make changes on the branch
- Query both main and branch to show isolation
- Either merge or discard the branch
- Use case: "What if I mark all tasks as done?"
-->

```bash
# TODO: Add branching examples
```

---

## What's Next?

You've learned the fundamentals of RaisinDB:

- **NodeTypes** define your data structure
- **Documents** are created, queried, and updated via REST or SQL
- **Branches** let you experiment safely

### Continue Learning

- [IoT Dashboard Tutorial](/docs/tutorials/iot-dashboard) - Handle time-series data and real-time updates
- [Shift Planner Tutorial](/docs/tutorials/shift-planner) - Model relationships and approval workflows
- [Concepts Deep Dive](/docs/why/concepts) - Understand revisions, branches, and more

---

## Complete Code

<details>
<summary>All commands from this tutorial</summary>

```bash
# TODO: Consolidate all curl commands here for easy copy-paste
```

</details>
