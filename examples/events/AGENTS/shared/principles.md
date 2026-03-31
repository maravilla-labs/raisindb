# Content-Driven Design Principles

## Everything Is a Content Node

All data in RaisinDB is stored as content nodes. A node has a type (NodeType),
properties (typed key-value pairs), and a position in a tree hierarchy. There are
no separate "database tables" -- the node tree IS the database.

## Schema-First via NodeTypes

Every node must conform to a NodeType schema. Define your data model in
`nodetypes/*.yaml` before creating content. The schema declares property names,
types, validation rules, and indexing. This ensures data integrity at the storage
layer.

## Workspace Isolation

Content lives inside workspaces. Each workspace declares which NodeTypes are
allowed and what the root folder structure looks like. Different workspaces can
serve different purposes (public site, admin data, user content) while sharing
the same NodeType definitions.

## Event-Driven with Triggers

Instead of polling for changes, subscribe to content events using triggers.
When a node is created, updated, or deleted, triggers fire and invoke handler
functions. This keeps logic decoupled and reactive.

## Flows Orchestrate Multi-Step Processes

For anything beyond a single function call, use flows. Flows coordinate
sequences of steps -- function calls, human tasks, AI interactions, waits, and
conditional branching. They persist state across steps and support compensation
(rollback) when things go wrong.

## Graph Relationships Connect Nodes

Nodes can reference each other using Reference properties. This creates a graph
of relationships on top of the tree hierarchy. Use References for cross-cutting
concerns like tags, categories, authors, and related content.
