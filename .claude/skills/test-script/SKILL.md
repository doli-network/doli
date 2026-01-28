---
name: test-script
description: Use this skill when you need to run, create, or modify test scripts. It checks scripts/README.md for existing scripts before creating new ones, and updates the registry when scripts are added or modified.
version: 1.0.0
---

# Test Script Management Skill

This skill manages test scripts in `scripts/` and keeps `scripts/README.md` synchronized.

## Git Author (Required)

All commits MUST use:
```
Name:  E. Weil
Email: weil@doli.network
```

## Workflow

### Step 1: Check Existing Scripts

**ALWAYS read `scripts/README.md` first** to understand what scripts already exist.

```bash
# Read the registry
cat scripts/README.md
```

Analyze the registry to determine if:
- An existing script already does what's needed → **use it**
- An existing script needs modification → **modify it**
- No suitable script exists → **create new one**

### Step 2: Decision Tree

```
Need to test functionality X?
│
├─ Check scripts/README.md
│  │
│  ├─ Script exists for X?
│  │  └─ YES → Run it: ./scripts/<script>.sh
│  │
│  ├─ Similar script exists?
│  │  └─ YES → Modify existing script
│  │           Update scripts/README.md
│  │           Commit both
│  │
│  └─ No suitable script?
│     └─ Create new script
│        Add entry to scripts/README.md
│        Test the script
│        Commit both
```

### Step 3: Creating a New Script

When creating a new test script:

1. **Create the script** in `scripts/` with proper conventions:
   ```bash
   #!/bin/bash
   # Description of what this script does
   #
   # Test scenario:
   # - What it sets up
   # - What it verifies

   set -e

   # Colors for output
   RED='\033[0;31m'
   GREEN='\033[0;32m'
   YELLOW='\033[1;33m'
   NC='\033[0m'

   # Cleanup trap
   cleanup() {
       echo -e "${YELLOW}Cleaning up...${NC}"
       # Kill spawned processes
   }
   trap cleanup EXIT

   # Script logic here...
   ```

2. **Make it executable:**
   ```bash
   chmod +x scripts/new_script.sh
   ```

3. **Test the script:**
   ```bash
   ./scripts/new_script.sh
   ```

4. **Update `scripts/README.md`** with a new entry following the existing format.

### Step 4: Updating the Registry

When adding or modifying a script, update `scripts/README.md`:

**For new scripts, add:**

```markdown
### script_name.sh

| Property | Value |
|----------|-------|
| **Path** | `scripts/script_name.sh` |
| **Purpose** | Brief description |
| **What it tests** | Specific functionality tested |
| **Dependencies** | Required binaries/crates |
| **Run time** | Approximate duration |
| **Output** | Where logs/data are saved |

**Usage:**
\`\`\`bash
./scripts/script_name.sh
\`\`\`

**Test scenario:**
- Bullet points describing the test
```

**Update the Quick Reference table:**

```markdown
| `script_name.sh` | N | ~X min | Purpose |
```

### Step 5: Commit

After script + README are ready and tested:

```bash
git add scripts/new_script.sh scripts/README.md
git commit --author="E. Weil <weil@doli.network>" -m "$(cat <<'EOF'
feat(scripts): add <script_name> for <purpose>

<Brief description of what the script tests>

Co-Authored-By: Claude Opus 4.5 <noreply@anthropic.com>
EOF
)"
```

## Script Categories

| Category | Naming Convention | Purpose |
|----------|-------------------|---------|
| Network | `launch_*.sh` | Start nodes/networks |
| Stress | `stress_*.sh` | Load/scalability tests |
| Reward | `test_*_rewards*.sh` | Validator reward testing |
| Integration | `test_*.sh` | General integration tests |
| Utility | `*.sh` | Helper scripts |

## Common Operations

### Run an existing script
```bash
./scripts/<script_name>.sh
```

### Check what scripts exist
```bash
cat scripts/README.md
# or
ls -la scripts/*.sh
```

### View script output
```bash
# Most scripts output to /tmp/doli-*/
tail -f /tmp/doli-*/logs/*.log
```

## Important Rules

1. **Always check README.md first** - avoid duplicate scripts
2. **Follow naming conventions** - makes scripts discoverable
3. **Include cleanup traps** - don't leave orphan processes
4. **Document dependencies** - others need to run these too
5. **Update README.md** - keep the registry accurate
6. **Test before committing** - verify the script works
7. **Commit script + README together** - atomic updates
