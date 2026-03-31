---
sidebar_position: 10
---

# FAQ

## General Questions

### What is RaisinDB?

RaisinDB is an open-source document database that combines NoSQL flexibility with git-like version control and structured schemas. It organizes data in tree structures and provides built-in versioning, branching, and schema validation.

### How is RaisinDB different from MongoDB?

While both are document databases, RaisinDB adds several unique features:

- **Tree Structure**: Data is organized hierarchically with parent-child relationships
- **Git-like Versioning**: Built-in commits, branches, tags, and history tracking (similar workflows to git, not the git protocol)
- **Schema Definitions**: YAML-based NodeTypes provide structure and validation
- **Multi-workspace**: Repository-based organization with isolated workspaces
- **Version Control**: Full audit trail and rollback capabilities

### Who should use RaisinDB?

RaisinDB is ideal for:

- **Content Management Systems** that need version control and hierarchical organization
- **Product Catalogs** with complex category structures and variant tracking
- **Knowledge Bases** requiring document versioning and cross-references
- **Applications** that need both flexibility and structure in their data model
- **Teams** that value data governance and change tracking

## Technical Questions

### What's the performance like?

RaisinDB is built on RocksDB for high-performance storage:

- **Read Performance**: O(log n) lookups with efficient indexing
- **Write Performance**: Atomic transactions with batch operations
- **Scalability**: Repository-based sharding for horizontal scaling
- **Concurrency**: Fine-grained locking for concurrent access

### Can I migrate from other databases?

Migration tools and guides are planned for:
- MongoDB collections to RaisinDB repositories
- PostgreSQL/MySQL relational data to hierarchical structures
- File-based content to RaisinDB nodes
- Custom migration scripts via the REST API

### How does the version control work?

RaisinDB's version control is inspired by Git:

- **Commits**: Atomic changesets with messages and timestamps
- **Branches**: Parallel development lines for different features
- **Tags**: Named points in history for releases or milestones
- **Merging**: Combine changes from different branches
- **History**: Full audit trail of all changes

### What about data consistency?

RaisinDB ensures data consistency through:

- **ACID Transactions**: All operations are atomic, consistent, isolated, and durable
- **Schema Validation**: NodeTypes enforce data structure and constraints
- **Referential Integrity**: Relationships between nodes are maintained
- **Isolation**: Workspaces prevent conflicting changes

## Deployment Questions

### What are the system requirements?

**Minimum Requirements:**
- 1 CPU core, 512MB RAM, 1GB storage
- Linux, macOS, or Windows
- Network access for API clients

**Production Recommendations:**
- 4+ CPU cores, 4GB+ RAM, SSD storage
- Linux server (Ubuntu 20.04+)
- Load balancer for high availability
- Backup and monitoring systems

### Can RaisinDB scale horizontally?

Yes, through several mechanisms:
- **Repository Sharding**: Distribute repositories across instances
- **Read Replicas**: Scale read operations with replicated data
- **Load Balancing**: Distribute HTTP requests across multiple instances
- **Storage Separation**: Use networked storage for shared data

### Is there a cloud offering?

Cloud offerings and managed services are planned for the future. Currently, you can deploy RaisinDB on any cloud provider using:
- Docker containers on Kubernetes
- Virtual machines with binary installations
- Serverless functions for specific workloads

## Development Questions

### What programming languages can I use?

RaisinDB provides a REST API that works with any language. Official SDKs are planned for:
- JavaScript/TypeScript
- Python  
- Go
- Rust

You can also use any HTTP client library in your preferred language.

### How do I define custom NodeTypes?

NodeTypes are defined using YAML schemas:

```yaml
name: my:CustomType
description: A custom node type
properties:
  - name: title
    type: String
    required: true
  - name: content
    type: Markdown
    required: false
allowed_children: ["raisin:Asset"]
versionable: true
publishable: true
```

Register them via the REST API:
```bash
POST /api/nodetypes/{repo}
```

### Can I extend RaisinDB with custom functionality?

Currently, customization is available through:
- **Custom NodeTypes** for structured data models
- **REST API integration** for custom business logic
- **Webhook support** (planned) for event-driven integrations
- **Plugin system** (planned) for extended functionality

## License Questions

### What license does RaisinDB use? {#license}

RaisinDB is licensed under the **Business Source License 1.1 (BSL 1.1)**.

**Key Terms:**
- **Licensor:** SOLUTAS GmbH
- **Change Date:** May 26, 2029
- **Change License:** Apache License 2.0 (after Change Date)

**What You Can Do:**
- ✅ Use RaisinDB freely in your applications
- ✅ Use in commercial products and services
- ✅ Modify and create derivative works
- ✅ Redistribute and make non-production use
- ✅ Use internally in your organization

**Usage Restrictions:**
- ❌ Cannot offer RaisinDB as a standalone hosted service or DBaaS
- ❌ Cannot embed into or use to build:
  - Content Management Systems (CMS)
  - Headless CMS platforms
  - Digital Experience Platforms (DXP)
  - Similar services where RaisinDB primarily stores/manages/delivers structured content

**After May 26, 2029:**
RaisinDB automatically converts to Apache License 2.0 with no restrictions.

### Can I use RaisinDB in my commercial product?

Yes, with some restrictions:

- ✅ **Embed in your application** - Use as the database for your product
- ✅ **Internal tools** - Build internal business applications
- ✅ **SaaS products** - Use as backend storage (if not CMS/DXP)
- ✅ **Mobile/Desktop apps** - Use for data persistence
- ❌ **Hosted CMS/DXP** - Cannot offer as a competing CMS/DXP platform
- ❌ **DBaaS** - Cannot offer as a standalone database service

For restricted use cases, contact SOLUTAS GmbH for a commercial license.

### Do I need to open source my application?

No. The BSL 1.1 is source-available, not copyleft:
- Your application code remains proprietary
- No requirement to release your source code
- Must display the BSL 1.1 license for RaisinDB
- Must comply with the Additional Use Grant restrictions

### What about enterprise support?

Enterprise support options are being developed:
- **Priority Support** - Direct access to maintainers
- **Custom Development** - Features tailored to your needs
- **Training and Consulting** - Expert guidance for your team
- **SLA Guarantees** - Response time commitments

Contact the maintainers for enterprise discussions.

## Community Questions

### How can I contribute?

Contributions are welcome in many forms:

- **Code Contributions**: Bug fixes, features, optimizations
- **Documentation**: Improve guides, examples, and API docs
- **Testing**: Report bugs, test new features, performance testing
- **Community**: Help other users, answer questions, share examples

See the [Contributing Guide](https://github.com/maravilla-labs/raisindb/blob/main/CONTRIBUTING.md) for details.

### Where can I get help?

- 📖 **Documentation**: Start with this documentation site
- 🐛 **GitHub Issues**: Report bugs and request features
- 💬 **Discussions**: Ask questions and share ideas
- 📧 **Direct Contact**: Reach out to maintainers for urgent issues

### Is there a roadmap?

Yes! Check out the [project roadmap](https://github.com/maravilla-labs/raisindb/blob/main/ROADMAP.md) for:
- Planned features and improvements
- Timeline estimates for major releases
- Community feedback and priorities
- Long-term vision and goals

## Still Have Questions?

- 📚 [Read the documentation](/docs/why/overview) for comprehensive guides
- 🐛 [Search existing issues](https://github.com/maravilla-labs/raisindb/issues) on GitHub
- 💡 [Create a new issue](https://github.com/maravilla-labs/raisindb/issues/new) for specific questions
- 📧 Contact the maintainers for urgent or private inquiries