# RaisinDB Stewardship System Documentation

This directory contains high-level documentation for the RaisinDB Stewardship System, designed for product owners, administrators, and non-technical stakeholders.

## What is Stewardship?

The Stewardship System allows authorized users (Stewards) to act on behalf of other users (Wards). Common scenarios include parents managing children's accounts, assistants handling tasks for executives, and temporary delegations with time limits.

## Documentation Structure

### 1. [Overview](./overview.md)
**Start here for a comprehensive introduction**

Learn about:
- What stewardship is and why it matters
- Key concepts: Steward, Ward, EntityCircle, RelationType
- How stewardship is derived from relationships
- Full vs scoped delegation
- What's included out of the box

**Audience:** Product owners, architects, decision makers

---

### 2. [Use Cases](./use-cases.md)
**Real-world scenarios with step-by-step flows**

Detailed examples including:
- **Household/Family Management**
  - Father creating children's accounts
  - Mother joining as co-parent
  - Managing school registrations
  - Children reaching adulthood
- **Organization Delegation**
  - Executive assistants with scoped access
  - Permission boundary enforcement
- **Temporary Access**
  - Vacation coverage with time limits
  - Automatic expiration
- **Legal Guardianship**
  - Non-parent guardians
  - Account management
- **Healthcare Scenarios**
  - Care coordinators managing patient accounts

**Audience:** Product managers, UX designers, business analysts

---

### 3. [Configuration Guide](./configuration.md)
**Admin guide for setting up and managing stewardship**

Topics covered:
- Enabling stewardship per repository
- Configuring settings:
  - Minor age threshold
  - Allowed workflows
  - Steward/ward limits
  - Consent requirements
- Managing relation types
- Managing entity circles
- Creating scoped delegations
- Monitoring and auditing
- Security best practices
- Troubleshooting

**Audience:** System administrators, repository managers

---

### 4. [Glossary](./glossary.md)
**Comprehensive terminology reference**

Alphabetically organized definitions with cross-references for:
- User roles (Steward, Ward, Guardian, etc.)
- Node types (StewardshipConfig, RelationType, EntityCircle)
- Relationships (PARENT_OF, GUARDIAN_OF, etc.)
- Delegation concepts (Full, Scoped, Time-Limited)
- Configuration settings
- Processes and workflows

**Audience:** All users, quick reference

---

## Quick Navigation

### I want to...

**Understand what stewardship is**
→ Start with [Overview](./overview.md)

**See how it works in practice**
→ Read [Use Cases](./use-cases.md)

**Set up stewardship for my repository**
→ Follow [Configuration Guide](./configuration.md)

**Look up a specific term**
→ Check [Glossary](./glossary.md)

**Implement stewardship features**
→ See developer documentation in `/docs/developer/stewardship/`

---

## Common Questions

### Is stewardship enabled by default?
No. Stewardship must be explicitly enabled in the repository configuration. See [Configuration Guide](./configuration.md#enable-stewardship).

### What's the difference between full and scoped delegation?
- **Full delegation**: Steward can perform any action the ward could
- **Scoped delegation**: Steward can only perform specific actions defined in permissions

See [Overview](./overview.md#delegation-types) for details.

### How do I set up parent-child relationships?
See [Use Cases - Household/Family Management](./use-cases.md#use-case-1-householdfamily-management) for step-by-step instructions.

### Can stewardship relationships expire automatically?
Yes. Use StewardshipOverrides with valid_until dates for time-limited delegations. See [Overview](./overview.md#time-limited-delegations) and [Configuration Guide](./configuration.md#creating-scoped-delegations).

### What happens when a child turns 18?
If PARENT_OF is configured with `require_minor_for_parent: true`, stewardship automatically ends when the ward reaches the minor age threshold. See [Use Cases - Child Reaching Adulthood](./use-cases.md#flow-1d-child-reaching-adulthood).

---

## Related Documentation

- **Developer Documentation**: `/docs/developer/stewardship/` - Technical implementation details
- **API Reference**: Technical API documentation for programmatic access
- **Access Control**: `/docs/features/access-control/` - Broader access control concepts
- **Row-Level Security**: `/docs/ROW_LEVEL_SECURITY.md` - How stewardship integrates with RLS

---

## Getting Help

- Review the [Glossary](./glossary.md) for terminology
- Check [Use Cases](./use-cases.md) for scenario-based guidance
- Consult [Configuration Guide](./configuration.md) troubleshooting section
- Contact RaisinDB support for complex configurations

---

## Document Status

**Version:** 1.0
**Last Updated:** December 2025
**Target Audience:** Product owners, administrators, non-technical stakeholders
**Feedback:** Please report issues or suggest improvements through your standard support channels
