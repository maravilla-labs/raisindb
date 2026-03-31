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
