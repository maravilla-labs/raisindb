---
sidebar_position: 2
---

# Permission Examples

Common permission patterns for different use cases.

## Multi-Tenant SaaS

Each organization can only see their own data:

```yaml
# Role: organization_member
permissions:
  - path: "**"
    operations: [read, create, update, delete]
    conditions:
      - property_equals:
          key: "organization_id"
          value: "$auth.organization_id"
```

## Content Management System

### Public Readers
Anonymous users can read published content:

```yaml
# Role: anonymous
permissions:
  - path: "content.**"
    operations: [read]
    conditions:
      - property_equals:
          key: "status"
          value: "published"
```

### Content Authors
Authors can edit their own articles:

```yaml
# Role: author
permissions:
  - path: "content.articles.**"
    operations: [read, create, update]
    conditions:
      - property_equals:
          key: "author"
          value: "$auth.user_id"

  # Authors can read all published content
  - path: "content.**"
    operations: [read]
    conditions:
      - property_equals:
          key: "status"
          value: "published"
```

### Editors
Editors can edit any content but not delete:

```yaml
# Role: editor
inherits: [author]
permissions:
  - path: "content.**"
    operations: [read, create, update]
    # No conditions - can edit anything
```

### Publishers
Publishers can publish/unpublish and delete content:

```yaml
# Role: publisher
inherits: [editor]
permissions:
  - path: "content.**"
    operations: [read, create, update, delete]
```

## E-Commerce

### Customers
Customers can view products and manage their own orders:

```yaml
# Role: customer
permissions:
  # Read all published products
  - path: "products.**"
    operations: [read]
    conditions:
      - property_equals:
          key: "status"
          value: "published"

  # View and manage own orders
  - path: "orders.**"
    operations: [read, update]
    conditions:
      - property_equals:
          key: "customer_id"
          value: "$auth.user_id"
    except_fields: [cost_price, supplier_notes]
```

### Store Managers
Managers can manage inventory but can't see customer payment info:

```yaml
# Role: store_manager
permissions:
  - path: "products.**"
    operations: [read, create, update, delete]

  - path: "orders.**"
    operations: [read, update]
    except_fields: [payment_details, card_last_four]
```

## Department Isolation

Each department can only access their own documents:

```yaml
# Role: marketing_member
permissions:
  - path: "departments.marketing.**"
    operations: [read, create, update, delete]

  - path: "shared.**"
    operations: [read]

# Role: engineering_member
permissions:
  - path: "departments.engineering.**"
    operations: [read, create, update, delete]

  - path: "shared.**"
    operations: [read]
```

## Subscription Tiers

### Free Tier
Limited access to public content:

```yaml
# Role: free_user
permissions:
  - path: "public.**"
    operations: [read]
```

### Premium Tier
Access to premium content and features:

```yaml
# Role: premium_user
inherits: [free_user]
permissions:
  - path: "premium.**"
    operations: [read]

  - path: "user_content.**"
    operations: [read, create, update]
    conditions:
      - property_equals:
          key: "owner_id"
          value: "$auth.user_id"
```

## Healthcare (HIPAA-like)

Patient records restricted to care team:

```yaml
# Role: doctor
permissions:
  - path: "patients.**"
    operations: [read, update]
    conditions:
      - property_in:
          key: "care_team"
          values: ["$auth.user_id"]
    except_fields: [billing_info, insurance_details]

# Role: billing_staff
permissions:
  - path: "patients.**"
    operations: [read]
    fields: [name, billing_info, insurance_details]
    # Can only see billing-related fields
```

## Hierarchical Approval

Documents require manager approval:

```yaml
# Role: employee
permissions:
  # Create drafts
  - path: "documents.**"
    operations: [create, read, update]
    conditions:
      - all:
        - property_equals:
            key: "author"
            value: "$auth.user_id"
        - property_in:
            key: "status"
            values: ["draft", "pending_approval"]

# Role: manager
inherits: [employee]
permissions:
  # Can approve team documents
  - path: "documents.**"
    operations: [read, update]
    conditions:
      - property_equals:
          key: "department"
          value: "$auth.department"
```

## Read-Only Auditors

Auditors can read everything but modify nothing:

```yaml
# Role: auditor
permissions:
  - path: "**"
    operations: [read]
    # No conditions - can read everything
    # No create/update/delete operations
```

## API Keys with Limited Scope

API keys for external integrations:

```yaml
# Role: external_api_readonly
permissions:
  - path: "products.**"
    operations: [read]
    fields: [id, name, price, description, sku]
    # Only expose public product fields

# Role: webhook_integration
permissions:
  - path: "orders.**"
    operations: [read]
    conditions:
      - property_equals:
          key: "status"
          value: "completed"
    # Only see completed orders
```

## Combining Multiple Conditions

Complex scenario: User can see article if:
- They are the author, OR
- Article is published AND in their department

```yaml
permissions:
  - path: "articles.**"
    operations: [read]
    conditions:
      - any:
        - property_equals:
            key: "author"
            value: "$auth.user_id"
        - all:
          - property_equals:
              key: "status"
              value: "published"
          - property_equals:
              key: "department"
              value: "$auth.department"
```

## Testing Permissions

Use admin impersonation to verify configurations:

```bash
# Test as a specific user
curl -X GET /api/nodes/content/articles \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "X-Raisin-Impersonate: user_123"

# Verify the user sees expected content
# Compare with another user
curl -X GET /api/nodes/content/articles \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "X-Raisin-Impersonate: user_456"
```
