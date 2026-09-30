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

### Repositories and languages

A repository's default language is set when it is created. Base content is
stored in that language, and every other language is a translation overlay on
top of it, so pick it before the first install:

```bash
raisindb repo create website --default-language de --languages de,fr,en
raisindb repo languages website                  # show default + supported languages
raisindb repo languages website --add it         # add a translation language
raisindb repo languages website --default de     # change the default language
raisindb repo list                               # LANGUAGES column per repository
```

Without `--default-language` the server uses `en`, and `--languages` alone is
refused rather than guessing which entry is the base. `--exists-ok` fails when
the existing repository has a different default language, and warns about
missing translation languages.

`repo languages --default <code>` changes the default language. The base
content is not rewritten; it is from then on treated as the new language. The
new default is added to the supported languages and the old one kept. The
server refuses while translations in the new language exist (they would collide
with the base content), and otherwise queues a full-text rebuild of every
branch and prints the job ids. It asks for confirmation at a terminal; pass
`--yes` in scripts. After adding languages, rebuild the full-text index
(`POST /api/admin/management/database/{tenant}/{repo}/fulltext/rebuild`).

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

## Server and Credentials

### Which server a command talks to

1. `-s, --server <url>` on the command
2. the `RAISINDB_SERVER` environment variable
3. `server` in `.raisinrc` (the server of the last `raisindb login`)
4. `http://localhost:8081`

`--server` applies to **every** request the command makes (for `deploy
--install`: the upload, the job stream, the install, the status polling and the
workspace reconcile). A login to another server never redirects it. `sync` uses
the server from `--server`, else `.raisindb-cli.yaml`, else the list above.

Every command that writes (`deploy`, `package upload`/`install`, `sync`,
`repo create`/`delete`, `secret`, `cors`, `ai provider set`, `user register`)
prints its target on stderr before the first write:

```text
→ http://localhost:8080 (repo studio, branch main)
```

### Which token is sent

1. the `RAISINDB_TOKEN` environment variable, to whatever server the command
   targets (an explicit choice)
2. `token` in `.raisinrc`, **only** when the target is the server stored next
   to it. Servers are compared after normalizing scheme and host case, default
   ports and trailing slashes (`https://Host:443/` equals `https://host`).

`.raisinrc` holds one login (one server, one token). When the target is a
different server, the CLI sends no `Authorization` header and says so:

```text
Note: the saved login is for https://prod.example.com, not http://localhost:8080; sending no credentials.
  Log in to it with: raisindb login -s http://localhost:8080   (or set RAISINDB_TOKEN)
```

`deploy`, `package upload`/`install`/`list` and `sync` stop at that point with
the same hint instead of failing later on a 401. Logging in to a second server
replaces the saved login; use `RAISINDB_TOKEN` to work with two servers from
one shell.

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
