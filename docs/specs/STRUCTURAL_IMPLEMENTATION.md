# Structural Implementation (Agent-Driven Workflow)

This section explains how to leverage `vecq` and `vecdb-asm` to move from "Design" to "Implementation" using AI Agents. This workflow bypasses the dangerous `write_to_file` sledgehammer in favor of **Schema-Driven Development**.

---

## 🏗️ The "Documentation-First" Loop

Instead of asking an Agent to "write code," you provide a **Structural Contract**. Because `vecq` generates documentation based on AST nodes, an Agent can use those same nodes as templates for new code.

### Step 1: Define the Contract
Create a Markdown stub for your new feature. Use the `vecq` standard signature format.

```rust
## terminal_buffer
**Signature**: `fn terminal_buffer(buf: &mut [u8], boids: &[Boid])`
**Description**: Maps float-based Boid coordinates to a 1D terminal byte buffer.
```

### Step 2: Invoke the Agent
Prompt your Agent:
> "Based on the `vecq` signature above, implement the logic in `src/render.rs`. Ensure you respect the AST structure and handle coordinate-to-index mapping."

### Step 3: Validate via AST
Once the Agent provides the code, use `vecq` to verify that the implementation matches the intended contract.

```bash
vecq src/render.rs -q '.functions[] | select(.name == "terminal_buffer")'
```

---

## 🛡️ Safety & Integrity Checks

When Agents implement "fake" functions from documentation, use these `vecq` audit queries to ensure code quality:

| Audit Type | Query | Goal |
|:---|:---|:---|
| **Mutability Check** | `select(.params[] | .is_mutable)` | Ensure `&mut` is only used where intended. |
| **Complexity Check** | `select(.metrics.complexity > 10)` | Catch Agents that write over-engineered spaghetti logic. |
| **Safety Check** | `select(.content | contains("unsafe"))` | Flag any "shortcuts" the Agent took in the implementation. |

---

## 💡 The "Sleipnir" Pro-Tip
By keeping your documentation and code in the same **AST-aware context**, you prevent the Agent from hallucinating incompatible types. If the documentation says `[Boid]`, the Agent is significantly less likely to try and use `Vec<Bird>`.