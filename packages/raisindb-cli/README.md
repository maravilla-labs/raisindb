# @raisindb/cli

Interactive command-line interface for RaisinDB with a beautiful Ink-based terminal UI.

## Features

- Interactive shell with command history and autocomplete
- SQL mode with syntax highlighting and multiline support
- Browser-based authentication with OAuth flow
- Package management (create and upload .rap packages)
- Beautiful gradient banner and intuitive UI
- Configuration file support (.raisinrc)

## Installation

```bash
npm install -g @raisindb/cli
# or
pnpm add -g @raisindb/cli
```

## Usage

### Interactive Shell (Default)

Start the interactive shell:

```bash
raisindb
# or
raisindb shell
```

Connect to a specific server:

```bash
raisindb shell --server http://localhost:8080
```

Connect and use a specific database:

```bash
raisindb shell --server http://localhost:8080 --database mydb
```

### Package Commands (Offline)

Create a .rap package from a folder:

```bash
raisindb package create ./my-package
raisindb package create ./my-package --output my-package.rap
```

Upload a package to the server:

```bash
raisindb package upload my-package.rap
raisindb package upload my-package.rap --server http://localhost:8080
```

Keep environment-specific values out of the YAML with `{env:...}` tokens:

```yaml
# content/stories/my-site/.node.yaml
properties:
  dev_url: "{env:PREVIEW_SERVER:-http://localhost:5173}"
```

```bash
raisindb package create ./my-package                          # uses the default
PREVIEW_SERVER=https://preview.example.ch \
  raisindb package create ./my-package                        # uses the env value
raisindb package create ./my-package --env production         # reads .env.production
```

Values resolve from `.env`, `.env.<profile>`, `.env.local`, `--env-file`, and
the process environment (highest precedence). `.env*` files are never packaged
or pushed. A token with no value and no inline default fails the command
instead of shipping a literal. The same substitution applies to
`package validate`, `deploy`, `sync --push/--watch`, and `.raisin-sync.yaml`.

### Type catalogs

Translation files are checked against the `translatable` markers of the
archetype or element type they belong to, including fields inherited through
`extends`. Types the package does not ship itself (`extends:
standard:ContentPage`, `archetype: standard:ArticlePage`, a `standard:Hero`
block) are resolved from type catalogs, loaded offline:

- **builtin** — RaisinDB's own node types, generated into
  `dist/builtin-types.json` when the CLI is built
  (`scripts/build-builtin-catalog.mjs`).
- **`~/.raisindb/catalogs/*.json`** — e.g. the Studio catalog
  (`studio-types.json`) that `maravilla update` or a build server downloads.
  The CLI never fetches one itself; files that are not a
  `raisin-type-catalog` v1 JSON document are ignored.

`package validate` names the catalogs in use. Without a Studio catalog, keys
inherited from `standard:*` types are not checked.

### Agent Skills

Scaffold an agent skill as a `SKILL.md` (frontmatter + Markdown) inside a
package. `deploy --install` and `sync --push` both install it as a
`raisin:Skill` node named after its folder.

```bash
raisindb create skill pdf-forms                          # asks who gets it
raisindb create skill pdf-forms --scope package          # content/functions/skills/pdf-forms/SKILL.md
raisindb create skill pdf-forms --scope local            # content/functions/local/skills/pdf-forms/SKILL.md
raisindb create skill pdf-forms --scope agent --path lib/acme/skills
```

| Scope | Folder | Who gets it |
|-------|--------|-------------|
| `package` | `functions:/skills/<name>` | every agent, shipped with the package |
| `local` | `functions:/local/skills/<name>` | every agent, this installation only |
| `agent` | `functions:/<path>/<name>` | only agents that list it in `skills:` |

An agent opts out of the two global layers with `global_skills: false`.

## Shell Commands

### Connection & Authentication

- `/connect <url>` - Connect to a RaisinDB server
- `/login` - Authenticate via browser (OAuth flow)
- `/logout` - Clear stored authentication

### Database Operations

- `use <database>` - Switch to a different database
- `/databases` - List available databases

### SQL Mode

- `/sql` - Enter SQL mode for running queries
- `/exit-sql` - Exit SQL mode (also available in SQL mode)

In SQL mode:
- Type queries normally and press Enter for single-line queries
- Omit semicolon to start multiline mode
- Press `Ctrl+Enter` to execute multiline queries
- Press `ESC` to cancel multiline input

### Package Management

- `/packages` - List installed packages
- `/install <name>` - Install a package by name (installs mixins before node types)
- `/upload [file]` - Upload a package file

### Other Commands

- `/help` - Show help screen with all commands
- `/clear` - Clear the terminal screen
- `/quit` or `/exit` - Exit the CLI

## Configuration File

The CLI looks for `.raisinrc` in the current directory tree, falling back to `~/.raisinrc`.

Example `.raisinrc`:

```yaml
server: http://localhost:8080
token: your-auth-token
default_repo: mydb
```

Configuration is automatically updated when you use `/connect`, `/login`, or `use` commands.

## Development

Build the package:

```bash
pnpm build
```

Run in development mode:

```bash
pnpm dev
```

Run the built CLI:

```bash
pnpm start
```

## Architecture

The CLI is built with:

- **Ink** - React for terminal UIs
- **Commander** - CLI argument parsing
- **ink-text-input** - Interactive text input
- **ink-gradient** & **ink-big-text** - Beautiful banner
- **sql-highlight** - SQL syntax highlighting
- **yaml** - Configuration file parsing
- **open** - Browser launching for OAuth

## License

MIT
