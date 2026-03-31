# TLA+ Tools Setup Guide

This guide helps you install and configure the TLA+ tools for verifying RaisinDB's replication protocol.

## Option 1: TLA+ Toolbox (Recommended for Beginners)

The TLA+ Toolbox is a GUI IDE for writing and checking TLA+ specifications.

### Installation

**macOS:**
```bash
# Download from GitHub releases
wget https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/TLAToolbox-1.8.0-macosx.cocoa.x86_64.zip

# Extract
unzip TLAToolbox-1.8.0-macosx.cocoa.x86_64.zip

# Move to Applications
mv TLA+ Toolbox.app /Applications/

# Launch
open "/Applications/TLA+ Toolbox.app"
```

**Linux:**
```bash
# Download
wget https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/TLAToolbox-1.8.0-linux.gtk.x86_64.zip

# Extract
unzip TLAToolbox-1.8.0-linux.gtk.x86_64.zip

# Run
cd toolbox
./toolbox
```

**Windows:**
```powershell
# Download from https://github.com/tlaplus/tlaplus/releases
# Extract and run TLA+ Toolbox.exe
```

### First Time Setup

1. **Create a new specification**:
   - File → Open Spec → Add New Spec
   - Browse to `formal/tla/VectorClock.tla`

2. **Create a model**:
   - TLC Model Checker → New Model
   - Name it "Model_1"

3. **Configure model parameters**:
   - What is the model?: Set constants
   - What to check?: Add invariants

4. **Run the model checker**:
   - Click "Run TLC on the model"

## Option 2: Command-Line Tools (Recommended for CI/CD)

### Installation

```bash
# Download TLA+ Tools JAR
wget https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar

# Move to a permanent location
mkdir -p ~/.tla
mv tla2tools.jar ~/.tla/

# Add alias to shell rc file (~/.bashrc, ~/.zshrc, etc.)
echo 'alias tlc="java -jar ~/.tla/tla2tools.jar"' >> ~/.zshrc

# Reload shell
source ~/.zshrc
```

### Java Requirement

TLA+ Tools require Java 11 or higher.

**Check Java version:**
```bash
java -version
```

**Install Java if needed:**

```bash
# macOS
brew install openjdk@17

# Ubuntu/Debian
sudo apt-get install openjdk-17-jdk

# Fedora/RHEL
sudo dnf install java-17-openjdk
```

### Running TLC Model Checker

**Basic usage:**
```bash
# Check a specification
tlc VectorClock.tla

# Run with specific model file
tlc MCRaisinReplication.tla
```

**Advanced options:**
```bash
# Use multiple workers (parallel model checking)
tlc -workers 8 MCRaisinReplication.tla

# Increase memory
tlc -Xmx4G MCRaisinReplication.tla

# Enable deadlock checking
tlc -deadlock MCRaisinReplication.tla

# Generate coverage statistics
tlc -coverage 1 MCRaisinReplication.tla

# Simulate (random walk instead of exhaustive)
tlc -simulate MCRaisinReplication.tla

# Continue from checkpoint
tlc -recover -continue MCRaisinReplication.tla
```

## Option 3: VS Code Extension (Recommended for Development)

### Installation

1. **Install VS Code extension**:
   - Open VS Code
   - Extensions → Search "TLA+"
   - Install "TLA+" by alygin

2. **Configure extension**:
   ```json
   // settings.json
   {
     "tlaplus.java.path": "/usr/bin/java",
     "tlaplus.tlc.modelChecker.workers": 8,
     "tlaplus.pluscal.options": "-termination"
   }
   ```

3. **Features**:
   - Syntax highlighting
   - Inline TLC error messages
   - Model checking integration
   - PlusCal transpilation

### Usage in VS Code

1. Open a `.tla` file
2. Right-click → "Check model with TLC"
3. View results in OUTPUT panel

## Verifying Installation

Create a simple test specification:

**test.tla:**
```tla
---- MODULE test ----
EXTENDS Naturals

VARIABLE x

Init == x = 0
Next == x' = x + 1

Inv == x < 100
=====================
```

**Run the model checker:**
```bash
tlc test.tla
```

**Expected output:**
```
TLC2 Version 2.18
Running in Model-Checking mode.
Parsing file test.tla
...
Model checking completed. No error has been found.
States found: 101
```

## Troubleshooting

### "java.lang.OutOfMemoryError"

Increase JVM heap size:
```bash
tlc -Xmx8G MCRaisinReplication.tla
```

### "Too many states to check"

1. **Reduce constants**: Make `MaxOps`, `MaxDelay` smaller
2. **Use symmetry**: Add SYMMETRY declarations
3. **Use simulation mode**: `-simulate` instead of exhaustive checking

### "TLC took too long"

1. **Use more workers**: `-workers 16`
2. **Use faster disk**: Move TLC temp files to SSD
3. **Simplify model**: Start with smaller state space, incrementally grow

## Next Steps

1. **Read TLA+ documentation**: https://lamport.azurewebsites.net/tla/tla.html
2. **Follow Learn TLA+ tutorial**: https://learntla.com/
3. **Study examples**: `formal/tla/` directory
4. **Watch video course**: Leslie Lamport's TLA+ Video Course

## Resources

- **TLA+ Home**: https://lamport.azurewebsites.net/tla/tla.html
- **Learn TLA+**: https://learntla.com/
- **TLA+ Community**: https://groups.google.com/g/tlaplus
- **Examples**: https://github.com/tlaplus/Examples
- **Awesome TLA+**: https://github.com/oskopek/awesome-tlaplus
