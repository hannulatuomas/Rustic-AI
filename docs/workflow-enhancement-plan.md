# Workflow Engine Enhancement Plan

**Purpose:** Extend Rustic-AI workflow engine with n8n-like features including grouped conditions, expression language, and new step types.

**Status:** Planning Phase
**Last Updated:** 2026-02-11

---

## Table of Contents

1. [Overview](#overview)
2. [Current State](#current-state)
3. [Phase 1: Grouped Conditions](#phase-1-grouped-conditions)
4. [Phase 2: Expression Language](#phase-2-expression-language)
5. [Phase 3: New Step Types](#phase-3-new-step-types)
6. [Edge Case Handling](#edge-case-handling)
7. [Backward Compatibility](#backward-compatibility)
8. [Implementation Order](#implementation-order)
9. [Testing Strategy](#testing-strategy)
10. [File Structure](#file-structure)

---

## Overview

### Goals

Enhance the workflow engine to support:

1. **Complex conditional logic** - Grouped conditions with AND/OR operators and nested groups
2. **Rich data transformations** - Expression language with transformations and aggregations
3. **Advanced workflow patterns** - Loops, merges, switches, and delays

### Success Criteria

- All features work with both JSON and YAML workflow definitions
- Backward compatible with existing workflows
- Clean error messages for edge cases
- Performance comparable to current implementation

---

## Current State

### What We Have

**Condition evaluation:**
- Single condition per condition step
- Expression-mode: `$step.check.value >= 10`, `$.foo matches "pattern"`
- Operators: `==`, `!=`, `>`, `>=`, `<`, `<=`, `contains`, `matches`, `truthy`, `falsy`
- Simple path extraction: `$step.check.value`

**Step routing:**
- Linear routing: `next`, `on_success`, `on_failure`
- No branching or looping
- No data merging

**Step kinds:**
- `Tool` - Execute a tool
- `Skill` - Run a skill
- `Agent` - Run an agent
- `Workflow` - Execute nested workflow
- `Condition` - Evaluate condition

### Limitations

1. **Cannot combine conditions** - Each condition step only evaluates one comparison
2. **No data transformation** - Cannot modify/format values in-place
3. **No iteration** - Cannot loop over arrays
4. **No multi-branch routing** - Cannot route based on value matching
5. **No data merging** - Cannot combine outputs from multiple steps

---

## Phase 1: Grouped Conditions

### 1.1 Type Definitions

**New Types:**

```rust
// File: rustic-ai-core/src/workflows/types.rs

/// Logical operator for combining conditions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogicalOperator {
    /// All conditions/groups must evaluate to true (AND)
    #[default]
    All,
    /// At least one condition/group must evaluate to true (OR)
    Any,
}

/// A single condition (leaf node in condition tree)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Condition {
    /// JSONPath to value to evaluate
    pub path: Option<String>,
    /// Expression to evaluate (alternative to path)
    pub expression: Option<String>,
    /// Comparison operator
    pub operator: ConditionOperator,
    /// Expected value for comparison
    pub value: Option<Value>,
}

/// A group of conditions (branch in condition tree)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConditionGroup {
    /// How to combine conditions/groups
    pub operator: LogicalOperator,
    /// Nested condition groups
    pub groups: Vec<ConditionGroup>,
    /// Leaf conditions
    pub conditions: Vec<Condition>,
}

impl Default for ConditionGroup {
    fn default() -> Self {
        Self {
            operator: LogicalOperator::default(),
            groups: Vec::new(),
            conditions: Vec::new(),
        }
    }
}
```

**Update WorkflowStep:**

```rust
// Extend existing WorkflowStep config support

// Backward compat: config.conditions = Vec<Condition>
// New: config.condition_group = ConditionGroup
```

### 1.2 Evaluation Logic

**Implementation in executor.rs:**

```rust
impl WorkflowExecutor {
    /// Evaluate a condition group recursively
    fn evaluate_condition_group(
        group: &ConditionGroup,
        outputs: &BTreeMap<String, Value>,
        step: &WorkflowStep,
    ) -> Result<bool> {
        // If both empty, treat as truthy (legacy behavior)
        if group.conditions.is_empty() && group.groups.is_empty() {
            return Ok(true);
        }

        let condition_results: Vec<bool> = group
            .conditions
            .iter()
            .map(|cond| Self::evaluate_condition_leaf(cond, outputs, step))
            .collect::<Result<_>>()?;

        let group_results: Vec<bool> = group
            .groups
            .iter()
            .map(|g| Self::evaluate_condition_group(g, outputs, step))
            .collect::<Result<_>>()?;

        match group.operator {
            LogicalOperator::All => {
                // All must be true
                Ok(condition_results.iter().all(|&b| b) &&
                   group_results.iter().all(|&b| b))
            }
            LogicalOperator::Any => {
                // At least one must be true
                Ok(condition_results.iter().any(|&b| b) ||
                   group_results.iter().any(|&b| b))
            }
        }
    }

    /// Evaluate a single leaf condition
    fn evaluate_condition_leaf(
        condition: &Condition,
        outputs: &BTreeMap<String, Value>,
        step: &WorkflowStep,
    ) -> Result<bool> {
        // Reuse existing evaluate_condition logic
        // Just construct a temporary WorkflowStep for compatibility
        let mut temp_config = serde_json::Map::new();

        if let Some(path) = &condition.path {
            temp_config.insert("path".to_string(), Value::String(path.clone()));
        }

        if let Some(expr) = &condition.expression {
            temp_config.insert("expression".to_string(), Value::String(expr.clone()));
        }

        temp_config.insert(
            "operator".to_string(),
            serde_json::to_value(condition.operator)
                .map_err(|e| Error::Tool(format!("Serialization error: {e}")))?,
        );

        if let Some(value) = &condition.value {
            temp_config.insert("value".to_string(), value.clone());
        }

        let temp_step = WorkflowStep {
            config: Value::Object(temp_config),
            ..step.clone()
        };

        Self::evaluate_condition(&temp_step, outputs)
    }
}
```

### 1.3 Validation

**Add to loader.rs:**

```rust
impl WorkflowLoader {
    fn validate_condition_group(
        group: &ConditionGroup,
        workflow: &WorkflowDefinition,
        step_id: &str,
        depth: usize,
        max_depth: usize,
    ) -> Result<()> {
        // Depth limit to prevent stack overflow
        if depth > max_depth {
            return Err(Error::Validation(format!(
                "workflow '{}' step '{}' has condition group depth {depth} exceeding maximum {max_depth}",
                workflow.name, step_id
            )));
        }

        // Validate no empty groups (unless root with default behavior)
        if group.conditions.is_empty() && group.groups.is_empty() && depth > 0 {
            return Err(Error::Validation(format!(
                "workflow '{}' step '{}' has empty nested condition group",
                workflow.name, step_id
            )));
        }

        // Validate conditions
        for condition in &group.conditions {
            if condition.path.is_none() && condition.expression.is_none() {
                return Err(Error::Validation(format!(
                    "workflow '{}' step '{}' condition must define path or expression",
                    workflow.name, step_id
                )));
            }
        }

        // Recursively validate nested groups
        for (index, nested) in group.groups.iter().enumerate() {
            Self::validate_condition_group(nested, workflow, step_id, depth + 1, max_depth)?;
        }

        Ok(())
    }
}
```

**Update step validation:**

```rust
// In validate_workflow() for each Condition step:
if step.kind == WorkflowStepKind::Condition {
    // Support legacy config.conditions OR new config.condition_group
    let has_legacy = step.config.get("conditions").is_some();
    let has_group = step.config.get("condition_group").is_some();

    if has_legacy && has_group {
        return Err(Error::Validation(format!(
            "workflow '{}' condition step '{}' cannot define both 'conditions' and 'condition_group'",
            workflow.name, step.id
        )));
    }

    if has_group {
        let group: ConditionGroup = serde_json::from_value(
            step.config.get("condition_group").cloned().unwrap_or_default()
        ).map_err(|e| Error::Validation(format!(
            "workflow '{}' step '{}' has invalid condition_group: {e}",
            workflow.name, step.id
        )))?;

        Self::validate_condition_group(&group, workflow, &step.id, 0, 5)?;
    }
}
```

---

## Phase 2: Expression Language

### 2.1 Design Principles

1. **Minimal but powerful** - Support common operations, not a full programming language
2. **No side effects** - Pure functional evaluation
3. **Composable** - Chain transformations: `$items.map(x => x.upper()).join(", ")`
4. **Safe defaults** - Graceful handling of null/undefined/empty

### 2.2 Expression Syntax

**Path-based expressions:**
```
$step.check.value
$data.users.0.name
$workflow.inputs.items
```

**Transformation expressions:**
```
$step.check.value.upper()
$step.check.value.lower()
$step.response.trim()
$step.text.split(", ")
$items.join(", ")
```

**Aggregation expressions:**
```
$items.sum()
$items.avg()
$items.map(x => x.price).min()
$items.filter(status == "active").count()
```

**Mixed expressions:**
```
$items.filter(x => x.active).map(x => x.name).upper().join(", ")
$step.response.total * 1.1
```

### 2.3 Function Library

#### String Functions

```rust
/// Transformations
upper(s: string) -> string
lower(s: string) -> string
trim(s: string) -> string
split(s: string, separator: string) -> array
join(arr: array, separator: string) -> string
replace(s: string, pattern: string, replacement: string) -> string
substring(s: string, start: number, end?: number) -> string
length(s: string) -> number

/// Regex
matches(s: string, pattern: string) -> boolean
replace_regex(s: string, pattern: string, replacement: string) -> string
```

#### Number Functions

```rust
abs(n: number) -> number
floor(n: number) -> number
ceil(n: number) -> number
round(n: number) -> number
```

#### Array Functions

```rust
/// Access
first(arr: array) -> any
last(arr: array) -> any
at(arr: array, index: number) -> any

/// Transformation
map(arr: array, lambda: (item) => any) -> array
filter(arr: array, lambda: (item) => boolean) -> array
flat_map(arr: array, lambda: (item) => array) -> array
take(arr: array, count: number) -> array
skip(arr: array, count: number) -> array
reverse(arr: array) -> array
sort(arr: array) -> array
unique(arr: array) -> array

/// Aggregation
sum(arr: array) -> number
avg(arr: array) -> number
min(arr: array) -> number
max(arr: array) -> number
count(arr: array) -> number
```

#### Object Functions

```rust
keys(obj: object) -> array<string>
values(obj: object) -> array
get(obj: object, key: string) -> any
has(obj: object, key: string) -> boolean
```

#### Type Functions

```rust
to_string(value: any) -> string
to_number(value: any) -> number
to_boolean(value: any) -> boolean
type(value: any) -> string  // "string", "number", "array", "object", "null"
```

### 2.4 Implementation Architecture

**New module: `rustic-ai-core/src/workflows/expressions.rs`**

```rust
use serde_json::Value;
use regex::Regex;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Expression {
    /// Simple path: $step.check.value
    Path(Path),
    /// Function call: upper(), sum(), etc.
    FunctionCall {
        name: String,
        args: Vec<Expression>,
    },
    /// Literal value: "hello", 42, true, null
    Literal(Value),
    /// Lambda for map/filter: x => x.upper()
    Lambda {
        param: String,
        body: Box<Expression>,
    },
    /// Binary operation: x + y
    BinaryOp {
        left: Box<Expression>,
        op: BinaryOperator,
        right: Box<Expression>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum BinaryOperator {
    Add, Sub, Mul, Div,
    Eq, Neq, Lt, Lte, Gt, Gte,
    And, Or,
}

#[derive(Debug, Clone)]
pub struct Path {
    /// Root context: "step", "input", "workflow", etc.
    root: Option<String>,
    /// Path components: ["check", "value"]
    segments: Vec<String>,
}

pub struct ExpressionEvaluator<'a> {
    outputs: &'a BTreeMap<String, Value>,
    // Optional: cache for evaluated sub-expressions
}

impl<'a> ExpressionEvaluator<'a> {
    pub fn new(outputs: &'a BTreeMap<String, Value>) -> Self {
        Self { outputs }
    }

    pub fn evaluate(&self, expr: &Expression) -> Result<Value, ExpressionError> {
        match expr {
            Expression::Path(path) => self.evaluate_path(path),
            Expression::Literal(v) => Ok(v.clone()),
            Expression::FunctionCall { name, args } => self.evaluate_function(name, args),
            Expression::Lambda { .. } => Err(ExpressionError::InvalidContext(
                "Lambdas can only be used within map/filter".to_string()
            )),
            Expression::BinaryOp { left, op, right } => {
                let l = self.evaluate(left)?;
                let r = self.evaluate(right)?;
                self.evaluate_binary_op(l, *op, r)
            }
        }
    }

    fn evaluate_path(&self, path: &Path) -> Result<Value, ExpressionError> {
        let root = if let Some(root) = &path.root {
            self.outputs.get(root)
        } else {
            Some(Value::Object(self.outputs.clone().into_iter().collect()))
        };

        let mut current = root.ok_or_else(|| ExpressionError::PathNotFound(
            path.segments.join(".")
        ))?;

        for segment in &path.segments {
            current = match current {
                Value::Array(arr) => {
                    // Try numeric index
                    if let Ok(index) = segment.parse::<usize>() {
                        arr.get(index).cloned().unwrap_or(Value::Null)
                    } else {
                        // Named property access on array elements
                        arr.first()
                            .and_then(|v| v.get(segment))
                            .cloned()
                            .unwrap_or(Value::Null)
                    }
                }
                Value::Object(map) => {
                    map.get(segment).cloned().unwrap_or(Value::Null)
                }
                _ => Value::Null,
            };
        }

        Ok(current)
    }
}
```

**Parser:**

```rust
pub fn parse_expression(input: &str) -> Result<Expression, ExpressionError> {
    // Simple recursive descent parser
    let tokens = tokenize(input)?;
    let (expr, remaining) = parse_expression(&tokens)?;

    if !remaining.is_empty() {
        return Err(ExpressionError::UnexpectedToken(format!(
            "Expected end of expression, got: {}",
            remaining[0]
        )));
    }

    Ok(expr)
}

#[derive(Debug, PartialEq)]
enum Token {
    Path(String),
    Literal(Value),
    Identifier(String),
    Dot,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Arrow,
    Comma,
    BinaryOp(BinaryOperator),
}

fn tokenize(input: &str) -> Result<Vec<Token>, ExpressionError> {
    // Tokenize: $, ., (, ), [, ], =>, ,, +, -, *, /, ==, !=, etc.
    // ...

    Ok(tokens)
}
```

### 2.5 Integration with Executor

**Replace render_value_with_outputs:**

```rust
impl WorkflowExecutor {
    fn render_expression(&self, expr_str: &str, outputs: &BTreeMap<String, Value>) -> Result<Value> {
        // Check if it's a simple path (backward compat)
        if !expr_str.contains('(') && !expr_str.contains(' ') {
            return Ok(Self::extract_path(
                &Self::outputs_root(outputs),
                expr_str
            ).unwrap_or(Value::Null));
        }

        // Parse and evaluate as expression
        let expr = parse_expression(expr_str)
            .map_err(|e| Error::Tool(format!("Expression parse error: {e}")))?;

        let evaluator = ExpressionEvaluator::new(outputs);
        evaluator.evaluate(&expr)
            .map_err(|e| Error::Tool(format!("Expression evaluation error: {e}")))
    }
}
```

---

## Phase 3: New Step Types

### 3.1 Wait/Delay Step

**Purpose:** Pause workflow execution for a specified duration or until a condition is met.

**Config Schema:**

```json
{
  "kind": "wait",
  "config": {
    "duration_seconds": 30,
    "until_expression": "$data.status == 'ready'",
    "check_interval_seconds": 5,
    "timeout_seconds": 300
  }
}
```

**Fields:**

- `duration_seconds` (optional): Fixed delay
- `until_expression` (optional): Wait until expression evaluates to true
- `check_interval_seconds` (optional, default: 5): Poll interval for until
- `timeout_seconds` (optional, default: 300): Maximum wait time

**Implementation:**

```rust
// In executor.rs, add WorkflowStepKind::Wait variant

async fn execute_wait_step(
    &self,
    step: &WorkflowStep,
    outputs: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    let duration = step.config.get("duration_seconds")
        .and_then(|v| v.as_u64());

    let until_expr = step.config.get("until_expression")
        .and_then(|v| v.as_str());

    if let Some(dur) = duration {
        // Simple delay
        tokio::time::sleep(tokio::time::Duration::from_secs(dur)).await;
        return Ok(BTreeMap::new());
    }

    if let Some(expr_str) = until_expr {
        // Conditional wait
        let interval = step.config.get("check_interval_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(5);

        let timeout = step.config.get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        let start = std::time::Instant::now();

        loop {
            let result = self.render_expression(expr_str, outputs)?;
            let is_ready = match result {
                Value::Bool(b) => b,
                _ => true,  // Non-falsy means ready
            };

            if is_ready {
                return Ok(BTreeMap::new());
            }

            if start.elapsed().as_secs() > timeout {
                return Err(Error::Tool(format!(
                    "Wait step '{}' timed out after {} seconds",
                    step.id, timeout
                )));
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;
        }
    }

    Err(Error::Tool(format!(
        "Wait step '{}' must specify duration_seconds or until_expression",
        step.id
    )))
}
```

### 3.2 Loop/Each Step

**Purpose:** Iterate over an array and execute logic for each item.

**Config Schema:**

```json
{
  "kind": "loop",
  "config": {
    "items": "$step.response.items",
    "item_variable": "item",
    "index_variable": "index",
    "parallel": false,
    "batch_size": 10,
    "body_step": "process_item",
    "max_iterations": 1000
  }
}
```

**Fields:**

- `items` (required): Expression yielding array
- `item_variable` (required): Variable name for current item
- `index_variable` (optional): Variable name for current index
- `parallel` (optional, default: false): Execute iterations in parallel
- `batch_size` (optional, default: 10): Parallel batch size
- `body_step` (required): Step ID to execute for each item
- `max_iterations` (optional, default: 1000): Safety limit

**Implementation:**

```rust
async fn execute_loop_step(
    &self,
    step: &WorkflowStep,
    outputs: &mut BTreeMap<String, Value>,
    workflow: &WorkflowDefinition,
    recursion_depth: usize,
) -> Result<BTreeMap<String, Value>> {
    let items_expr = step.config.get("items")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Tool(format!(
            "Loop step '{}' missing 'items' config",
            step.id
        )))?;

    let items_value = self.render_expression(items_expr, outputs)?;
    let items = items_value.as_array()
        .ok_or_else(|| Error::Tool(format!(
            "Loop step '{}' 'items' expression did not return array: {:?}",
            step.id, items_value
        )))?;

    if items.is_empty() {
        // Empty array: return empty results
        return Ok(BTreeMap::new());
    }

    let item_var = step.config.get("item_variable")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Tool(format!(
            "Loop step '{}' missing 'item_variable' config",
            step.id
        )))?;

    let index_var = step.config.get("index_variable")
        .and_then(|v| v.as_str());

    let body_step_id = step.config.get("body_step")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Tool(format!(
            "Loop step '{}' missing 'body_step' config",
            step.id
        )))?;

    let max_iterations = step.config.get("max_iterations")
        .and_then(|v| v.as_usize())
        .unwrap_or(1000);

    if items.len() > max_iterations {
        return Err(Error::Tool(format!(
            "Loop step '{}' has {} items exceeding max_iterations {}",
            step.id, items.len(), max_iterations
        )));
    }

    let is_parallel = step.config.get("parallel")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let batch_size = step.config.get("batch_size")
        .and_then(|v| v.as_usize())
        .unwrap_or(10);

    let mut results = Vec::new();

    if is_parallel {
        // Parallel execution in batches
        for chunk in items.chunks(batch_size) {
            let mut tasks = Vec::new();

            for (index, item) in chunk.iter().enumerate() {
                // Create new output scope for this iteration
                let mut iteration_outputs = outputs.clone();

                iteration_outputs.insert(
                    format!("loop.{item_var}"),
                    item.clone()
                );

                if let Some(idx_var) = index_var {
                    iteration_outputs.insert(
                        format!("loop.{idx_var}"),
                        Value::Number((index as u64).into())
                    );
                }

                let workflow = workflow.clone();
                let executor = self.clone();
                let body_id = body_step_id.to_string();

                tasks.push(tokio::spawn(async move {
                    executor.execute_workflow_step(
                        &workflow,
                        &body_id,
                        &iteration_outputs,
                        recursion_depth + 1
                    ).await
                }));
            }

            // Wait for batch and collect results
            for task in tasks {
                let result = task.await
                    .map_err(|e| Error::Tool(format!(
                        "Loop task join error: {e}"
                    )))?
                    .map_err(|e| Error::Tool(format!(
                        "Loop iteration error: {e}"
                    )))?;

                results.push(result);
            }
        }
    } else {
        // Sequential execution
        for (index, item) in items.iter().enumerate() {
            outputs.insert(
                format!("loop.{item_var}"),
                item.clone()
            );

            if let Some(idx_var) = index_var {
                outputs.insert(
                    format!("loop.{idx_var}"),
                    Value::Number((index as u64).into())
                );
            }

            let result = self.execute_workflow_step(
                workflow,
                body_step_id,
                outputs,
                recursion_depth + 1
            ).await?;

            results.push(result);
        }
    }

    // Return all results as array
    let mut final_outputs = BTreeMap::new();
    final_outputs.insert(
        "results".to_string(),
        Value::Array(results)
    );

    // Clean up loop variables
    outputs.retain(|k, _| !k.starts_with("loop."));

    Ok(final_outputs)
}
```

### 3.3 Merge Step

**Purpose:** Combine data from multiple sources.

**Config Schema:**

```json
{
  "kind": "merge",
  "config": {
    "mode": "merge",
    "inputs": {
      "data1": "$step1.outputs",
      "data2": "$step2.outputs"
    }
  }
}
```

**Modes:**

1. **merge**: Combine objects (later values overwrite earlier)
2. **append**: Concatenate arrays
3. **combine**: Create object with all inputs as properties
4. **multiplex**: Distribute single input to all outputs (for multi-branch workflows)

**Implementation:**

```rust
async fn execute_merge_step(
    &self,
    step: &WorkflowStep,
    outputs: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    let mode = step.config.get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("merge");

    let inputs_obj = step.config.get("inputs")
        .and_then(|v| v.as_object())
        .ok_or_else(|| Error::Tool(format!(
            "Merge step '{}' missing 'inputs' object",
            step.id
        )))?;

    // Evaluate all input expressions
    let mut evaluated_inputs = BTreeMap::new();
    for (key, value_expr) in inputs_obj {
        let expr_str = value_expr.as_str().ok_or_else(|| Error::Tool(format!(
            "Merge step '{}' input '{}' is not a string expression",
            step.id, key
        )))?;

        evaluated_inputs.insert(
            key.clone(),
            self.render_expression(expr_str, outputs)?
        );
    }

    let result = match mode {
        "merge" => {
            // Combine objects
            let mut merged = serde_json::Map::new();
            for (key, value) in evaluated_inputs {
                match value {
                    Value::Object(map) => {
                        merged.extend(map);
                    }
                    _ => {
                        return Err(Error::Tool(format!(
                            "Merge step '{}' input '{}' is not an object for 'merge' mode",
                            step.id, key
                        )));
                    }
                }
            }
            Value::Object(merged)
        }

        "append" => {
            // Concatenate arrays
            let mut all_items = Vec::new();
            for (key, value) in &evaluated_inputs {
                if let Some(arr) = value.as_array() {
                    all_items.extend(arr.clone());
                } else {
                    return Err(Error::Tool(format!(
                        "Merge step '{}' input '{}' is not an array for 'append' mode",
                        step.id, key
                    )));
                }
            }
            Value::Array(all_items)
        }

        "combine" => {
            // Create object with all inputs as properties
            let mut combined = serde_json::Map::new();
            for (key, value) in evaluated_inputs {
                combined.insert(key, value);
            }
            Value::Object(combined)
        }

        "multiplex" => {
            // Return evaluated inputs directly
            Value::Object(
                evaluated_inputs.into_iter().collect()
            )
        }

        other => {
            return Err(Error::Tool(format!(
                "Merge step '{}' has unknown mode '{}'",
                step.id, other
            )));
        }
    };

    let mut final_outputs = BTreeMap::new();
    final_outputs.insert("merged".to_string(), result);
    Ok(final_outputs)
}
```

### 3.4 Switch Step

**Purpose:** Route to different branches based on value matching.

**Config Schema:**

```json
{
  "kind": "switch",
  "config": {
    "value": "$step.status",
    "cases": [
      {
        "value": "active",
        "next": "handle_active"
      },
      {
        "value": "inactive",
        "next": "handle_inactive"
      },
      {
        "pattern": "^pending",
        "next": "handle_pending"
      }
    ],
    "default": "handle_default"
  }
}
```

**Fields:**

- `value` (required): Expression to evaluate
- `cases` (required): Array of case definitions
  - `value`: Exact match value
  - `pattern`: Regex pattern (alternative to value)
  - `next`: Step ID to route to
- `default` (required): Step ID if no case matches

**Implementation:**

```rust
async fn execute_switch_step(
    &self,
    step: &WorkflowStep,
    outputs: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    let value_expr = step.config.get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Tool(format!(
            "Switch step '{}' missing 'value' expression",
            step.id
        )))?;

    let value = self.render_expression(value_expr, outputs)?;

    let cases_arr = step.config.get("cases")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Error::Tool(format!(
            "Switch step '{}' missing 'cases' array",
            step.id
        )))?;

    let default_next = step.config.get("default")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Tool(format!(
            "Switch step '{}' missing 'default' step",
            step.id
        )))?;

    // Try to find matching case
    for case_obj in cases_arr {
        let case = case_obj.as_object()
            .ok_or_else(|| Error::Tool(format!(
                "Switch step '{}' case is not an object",
                step.id
            )))?;

        // Check for exact match
        if let Some(expected) = case.get("value") {
            if value == *expected {
                let next_step = case.get("next")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::Tool(format!(
                        "Switch step '{}' case missing 'next'",
                        step.id
                    )))?;

                // Return routing instruction
                let mut result = BTreeMap::new();
                result.insert(
                    "_route".to_string(),
                    Value::String(next_step.to_string())
                );
                result.insert(
                    "_matched_value".to_string(),
                    value
                );
                return Ok(result);
            }
        }

        // Check for pattern match
        if let Some(pattern) = case.get("pattern").and_then(|v| v.as_str()) {
            let value_str = value.as_str().unwrap_or("");
            let regex = Regex::new(pattern).map_err(|e| Error::Tool(format!(
                "Switch step '{}' has invalid regex '{}': {}",
                step.id, pattern, e
            )))?;

            if regex.is_match(value_str) {
                let next_step = case.get("next")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::Tool(format!(
                        "Switch step '{}' case missing 'next'",
                        step.id
                    )))?;

                let mut result = BTreeMap::new();
                result.insert(
                    "_route".to_string(),
                    Value::String(next_step.to_string())
                );
                result.insert(
                    "_matched_pattern".to_string(),
                    Value::String(pattern.to_string())
                );
                result.insert(
                    "_matched_value".to_string(),
                    value
                );
                return Ok(result);
            }
        }
    }

    // No match found, use default
    let mut result = BTreeMap::new();
    result.insert(
        "_route".to_string(),
        Value::String(default_next.to_string())
    );
    result.insert(
        "_matched".to_string(),
        Value::Bool(false)
    );
    Ok(result)
}
```

**Routing Integration:**

```rust
// In execute_workflow_step(), after step execution:
if step.kind == WorkflowStepKind::Switch {
    if let Some(route) = step_outputs.get("_route").and_then(|v| v.as_str()) {
        return Ok(route.to_string());
    }
}
```

---

## Edge Case Handling

### 1. Empty Arrays

**Scenarios:**
- Loop over empty array
- Function called on empty array (`sum()`, `avg()`)
- Merge with empty array inputs

**Handling Strategy:**

```rust
// Loop over empty array
if items.is_empty() {
    tracing::debug!("Loop step '{}' processing empty array", step.id);
    return Ok(BTreeMap::new());  // Return empty results, don't execute body
}

// Aggregation on empty array
fn evaluate_sum(&self, arr: &[Value]) -> Result<Value> {
    if arr.is_empty() {
        return Ok(Value::Number(0.into()));  // Neutral element for sum
    }
    // ... sum logic
}

fn evaluate_avg(&self, arr: &[Value]) -> Result<Value> {
    if arr.is_empty() {
        return Err(ExpressionError::InvalidOperation(
            "Cannot calculate average of empty array".to_string()
        ));
    }
    // ... avg logic
}

// Merge with empty array
if value.as_array().map(|a| a.is_empty()).unwrap_or(false) {
    // Skip this input in append/merge modes
    continue;
}
```

### 2. Null/Undefined Values

**Scenarios:**
- Path doesn't exist: `$step.missing.value`
- Null in array: `$items.filter(x => x.active)` where `x.active` is null
- Null comparisons: `null == null`, `null > 0`

**Handling Strategy:**

```rust
// Path extraction returns Null for missing paths
fn evaluate_path(&self, path: &Path) -> Result<Value, ExpressionError> {
    let mut current = root.ok_or_else(|| ExpressionError::PathNotFound(...))?;

    for segment in &path.segments {
        current = match current {
            Value::Object(map) => map.get(segment).cloned().unwrap_or(Value::Null),
            Value::Array(arr) => {
                if let Ok(index) = segment.parse::<usize>() {
                    arr.get(index).cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                }
            }
            _ => Value::Null,
        };
    }

    Ok(current)  // Never error, return Null for missing
}

// Null-aware comparisons
fn compare_values(a: &Value, b: &Value, op: BinaryOperator) -> Result<bool> {
    match (a, b) {
        (Value::Null, Value::Null) => Ok(matches!(op, BinaryOperator::Eq)),
        (Value::Null, _) | (_, Value::Null) => Ok(matches!(op, BinaryOperator::Neq)),
        _ => normal_comparison(a, b, op),
    }
}

// Null filtering
fn evaluate_filter(&self, arr: &[Value], lambda: &Lambda) -> Result<Vec<Value>> {
    arr.iter()
        .filter(|item| {
            let result = self.evaluate_lambda(lambda, item);
            match result {
                Ok(Value::Bool(true)) => true,
                Ok(Value::Bool(false)) => false,
                // Null or non-boolean: treat as falsy
                Ok(_) => false,
                Err(_) => false,  // Error in predicate, skip item
            }
        })
        .cloned()
        .collect()
}
```

**Configuration for strictness:**

```rust
// In config, add:
{
  "workflows": {
    "null_handling": "strict",  // or "lenient" (default)
  }
}

// Strict mode: error on null path access
// Lenient mode: return null for missing paths
```

### 3. Deep Nesting

**Scenarios:**
- Condition groups nested 20+ levels deep
- Expression chains: `$items.map(x => x.nested.deep.value)`
- Recursive workflows calling each other

**Handling Strategy:**

```rust
// Configurable depth limits
const MAX_CONDITION_DEPTH: usize = 5;
const MAX_EXPRESSION_DEPTH: usize = 10;
const MAX_WORKFLOW_RECURSION: usize = 16;  // Already exists

// Validate during workflow loading
fn validate_condition_group(group: &ConditionGroup, depth: usize) -> Result<()> {
    if depth > MAX_CONDITION_DEPTH {
        return Err(Error::Validation(format!(
            "Condition group depth {} exceeds maximum {}",
            depth, MAX_CONDITION_DEPTH
        )));
    }

    for nested in &group.groups {
        Self::validate_condition_group(nested, depth + 1)?;
    }

    Ok(())
}

// Expression parsing depth limit
fn parse_expression(tokens: &[Token], depth: usize) -> Result<Expression> {
    if depth > MAX_EXPRESSION_DEPTH {
        return Err(ExpressionError::DepthExceeded(
            format!("Expression depth {} exceeds maximum {}", depth, MAX_EXPRESSION_DEPTH)
        ));
    }

    // ... parse logic
}

// Workflow recursion enforcement
async fn execute_workflow(
    &self,
    request: &WorkflowRunRequest,
) -> Result<WorkflowExecutionResult> {
    if request.recursion_depth > self.config.max_recursion_depth.unwrap_or(16) {
        return Err(Error::Tool(format!(
            "Workflow recursion depth {} exceeds maximum {}",
            request.recursion_depth,
            self.config.max_recursion_depth.unwrap_or(16)
        )));
    }

    // ... execute logic
}
```

### 4. Circular References

**Scenarios:**
- Workflow step references itself: `{ "id": "step1", "next": "step1" }`
- Mutual recursion: Workflow A calls B, B calls A
- Expression circular dependency (if we support references to expression results)

**Handling Strategy:**

```rust
// Detect cycles in step graph during loading
fn validate_workflow_step_graph(workflow: &WorkflowDefinition) -> Result<()> {
    let mut visited = HashSet::new();
    let mut stack = Vec::new();

    for (name, entrypoint) in &workflow.entrypoints {
        if let Err(e) = Self::detect_cycle(
            &entrypoint.step,
            workflow,
            &mut visited,
            &mut stack,
        ) {
            return Err(Error::Validation(format!(
                "Workflow '{}' has cycle in step graph: {}",
                workflow.name, e
            )));
        }
    }

    Ok(())
}

fn detect_cycle(
    step_id: &str,
    workflow: &WorkflowDefinition,
    visited: &mut HashSet<String>,
    stack: &mut Vec<String>,
) -> Result<()> {
    if stack.contains(&step_id.to_string()) {
        // Found cycle
        let cycle_path = stack.iter()
            .skip_while(|s| s != &step_id.to_string())
            .cloned()
            .chain(Some(step_id.to_string()))
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(format!("Cycle detected: {}", cycle_path));
    }

    if visited.contains(step_id) {
        return Ok(());  // Already validated
    }

    visited.insert(step_id.to_string());
    stack.push(step_id.to_string());

    // Get step and check all outgoing edges
    if let Some(step) = workflow.steps.iter().find(|s| s.id == step_id) {
        for target in [&step.next, &step.on_success, &step.on_failure]
            .into_iter()
            .flatten()
        {
            Self::detect_cycle(target, workflow, visited, stack)?;
        }
    }

    stack.pop();
    Ok(())
}

// Workflow mutual recursion detection
struct WorkflowExecutor {
    // Track active workflow stack
    active_workflows: Arc<Mutex<Vec<String>>>,
}

async fn execute_workflow(
    &self,
    request: &WorkflowRunRequest,
) -> Result<WorkflowExecutionResult> {
    let mut active = self.active_workflows.lock().await;

    // Check for mutual recursion
    if request.recursion_depth > 0 {
        let previous_count = active.iter()
            .filter(|w| w == &request.workflow_name)
            .count();

        if previous_count > 0 {
            drop(active);
            return Err(Error::Tool(format!(
                "Mutual recursion detected: workflow '{}' already active at depth {}",
                request.workflow_name, request.recursion_depth
            )));
        }
    }

    active.push(request.workflow_name.clone());
    drop(active);

    // Execute workflow...

    // Remove from active stack
    let mut active = self.active_workflows.lock().await;
    active.pop();

    Ok(result)
}
```

### 5. Large Data Handling

**Scenarios:**
- Loop over 10,000 items
- Expression with huge array: `$hugeArray.map(...).sum()`
- Merge step with many inputs

**Handling Strategy:**

```rust
// Configurable iteration limits
const DEFAULT_MAX_ITERATIONS: usize = 1000;
const DEFAULT_MAX_ARRAY_SIZE: usize = 100_000;

// Validate during step execution
fn validate_loop_size(items: &[Value], max: usize) -> Result<()> {
    if items.len() > max {
        return Err(Error::Tool(format!(
            "Array size {} exceeds maximum {}",
            items.len(), max
        )));
    }
    Ok(())
}

// Streaming-friendly aggregation (for future)
fn evaluate_sum_streaming(
    &self,
    arr: &[Value],
) -> Result<Value> {
    // Process incrementally, avoid holding intermediate copies
    let mut sum: f64 = 0.0;
    for item in arr {
        match item {
            Value::Number(n) => sum += n.as_f64().unwrap_or(0.0),
            _ => {}  // Skip non-numeric
        }
    }
    Ok(Value::Number(sum.into()))
}
```

### 6. Error Propagation

**Scenarios:**
- Step errors with `continue_on_error`
- Loop iteration fails
- Condition evaluation error

**Handling Strategy:**

```rust
// Step error handling
async fn execute_workflow_step(
    &self,
    workflow: &WorkflowDefinition,
    step_id: &str,
    outputs: &BTreeMap<String, Value>,
    recursion_depth: usize,
) -> Result<(BTreeMap<String, Value>, Option<String>)> {
    let step = workflow.steps.iter()
        .find(|s| s.id == step_id)
        .ok_or_else(|| Error::Tool(format!(
            "Step '{}' not found in workflow '{}'",
            step_id, workflow.name
        )))?;

    match self.execute_step_by_kind(step, outputs, workflow, recursion_depth).await {
        Ok(result) => Ok((result, step.on_success.clone())),
        Err(err) => {
            if step.continue_on_error {
                tracing::warn!(
                    "Step '{}' failed but continue_on_error is true: {}",
                    step.id, err
                );

                // Return error result with _error field
                let mut error_result = BTreeMap::new();
                error_result.insert(
                    "_error".to_string(),
                    Value::String(err.to_string())
                );
                error_result.insert(
                    "_error_kind".to_string(),
                    Value::String("step_error".to_string())
                );
                Ok((error_result, step.on_failure.clone()))
            } else {
                Err(err)
            }
        }
    }
}

// Loop iteration error handling
for (index, item) in items.iter().enumerate() {
    match self.execute_iteration(item, index).await {
        Ok(result) => results.push(result),
        Err(err) => {
            tracing::error!(
                "Loop iteration {} failed: {}",
                index, err
            );

            if step.continue_on_error {
                // Add error result and continue
                let mut error_result = BTreeMap::new();
                error_result.insert(
                    "_error".to_string(),
                    Value::String(err.to_string())
                );
                results.push(error_result);
            } else {
                // Fail entire loop
                return Err(err);
            }
        }
    }
}

// Condition evaluation error handling
fn evaluate_condition_group(
    group: &ConditionGroup,
    outputs: &BTreeMap<String, Value>,
    step: &WorkflowStep,
) -> Result<bool> {
    let mut errors = Vec::new();

    for condition in &group.conditions {
        match Self::evaluate_condition_leaf(condition, outputs, step) {
            Ok(result) => result,
            Err(e) => {
                errors.push(e.to_string());
                false  // Treat error as falsy
            }
        };
    }

    if !errors.is_empty() {
        tracing::warn!(
            "Condition group evaluation had errors: {:?}",
            errors
        );
    }

    // Combine results based on operator
    // ...
}
```

---

## Backward Compatibility

### Condition Steps

**Legacy format (still supported):**

```json
{
  "id": "check_status",
  "kind": "condition",
  "config": {
    "path": "$step.response.status",
    "operator": "equals",
    "value": "active"
  },
  "next": "process"
}
```

**New format:**

```json
{
  "id": "check_status",
  "kind": "condition",
  "config": {
    "condition_group": {
      "operator": "all",
      "conditions": [
        {
          "path": "$step.response.status",
          "operator": "equals",
          "value": "active"
        },
        {
          "path": "$step.response.priority",
          "operator": "greater_than",
          "value": 5
        }
      ]
    }
  },
  "next": "process"
}
```

**Loader logic:**

```rust
fn parse_condition_config(config: &Value) -> Result<ConditionEvaluationMode> {
    if let Some(group) = config.get("condition_group") {
        let group: ConditionGroup = serde_json::from_value(group.clone())
            .map_err(|e| Error::Validation(format!(
                "Invalid condition_group: {e}"
            )))?;
        Ok(ConditionEvaluationMode::Grouped(group))
    } else if let Some(_conditions) = config.get("conditions") {
        // Legacy format
        let conditions: Vec<Condition> = serde_json::from_value(
            config.get("conditions").cloned().unwrap_or(Value::Array(vec![]))
        ).map_err(|e| Error::Validation(format!(
            "Invalid conditions array: {e}"
        )))?;

        let group = ConditionGroup {
            operator: LogicalOperator::All,  // Legacy uses AND
            conditions,
            groups: vec![],
        };
        Ok(ConditionEvaluationMode::Grouped(group))
    } else {
        // Single condition format
        let condition = Condition {
            path: config.get("path").and_then(|v| v.as_str().map(String::from)),
            expression: config.get("expression").and_then(|v| v.as_str().map(String::from)),
            operator: config.get("operator")
                .and_then(|v| v.as_str())
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(ConditionOperator::Exists),
            value: config.get("value").cloned(),
        };

        let group = ConditionGroup {
            operator: LogicalOperator::All,
            conditions: vec![condition],
            groups: vec![],
        };
        Ok(ConditionEvaluationMode::Grouped(group))
    }
}
```

### Expression Compatibility

**Simple paths (no change):**

```rust
// These still work exactly as before:
"$step.check.value"
"$workflow.inputs.data"
"$output"
```

**Expression detection:**

```rust
fn is_expression(input: &str) -> bool {
    // Has function call
    if input.contains('(') && input.contains(')') {
        return true;
    }

    // Has operators (not just path)
    if matches!(input, r"\s*[+\-*/=<>]\s*") {
        return true;
    }

    // Has whitespace (likely expression)
    if input.contains(' ') {
        return true;
    }

    false
}

fn render_value(&self, value: &Value, outputs: &BTreeMap<String, Value>) -> Value {
    match value {
        Value::String(s) if s.starts_with('$') && is_expression(s) => {
            // Use expression evaluator
            match self.render_expression(s, outputs) {
                Ok(result) => result,
                Err(e) => {
                    tracing::error!("Expression error: {}", e);
                    Value::Null
                }
            }
        }
        Value::String(s) if s.starts_with('$') => {
            // Use simple path extraction (backward compat)
            Self::extract_path(&Self::outputs_root(outputs), s)
                .unwrap_or(Value::Null)
        }
        _ => value.clone(),
    }
}
```

---

## Implementation Order

### Phase 1: Grouped Conditions (Priority 1)

**Estimated effort:** 2-3 days

**Tasks:**
1. Add `LogicalOperator`, `Condition`, `ConditionGroup` types to `types.rs`
2. Implement `evaluate_condition_group()` in `executor.rs`
3. Add cycle detection to `loader.rs`
4. Add depth validation to `loader.rs`
5. Update CLI to display condition groups

**Dependencies:** None
**Risks:** Medium (logic complexity, edge cases)

### Phase 2: Expression Language (Priority 2)

**Estimated effort:** 3-4 days

**Tasks:**
1. Implement tokenizer in `expressions.rs`
2. Implement parser in `expressions.rs`
3. Implement evaluator with function library
4. Integrate into executor (replace `render_value_with_outputs`)
5. Add error handling and edge case support
6. Update config schema for expression syntax

**Dependencies:** None
**Risks:** High (parser complexity, performance, edge cases)

**Sub-phases:**
- 2a: Path + literals + basic functions (upper, lower, length)
- 2b: Array functions (map, filter, sum, etc.)
- 2c: Advanced features (lambdas, aggregations)

### Phase 3: New Step Types (Priority 3)

#### 3.1 Wait Step (1 day)
**Tasks:**
1. Add `WorkflowStepKind::Wait` variant
2. Implement `execute_wait_step()`
3. Add validation for wait config

#### 3.2 Loop Step (2-3 days)
**Tasks:**
1. Add `WorkflowStepKind::Loop` variant
2. Implement `execute_loop_step()` with sequential mode
3. Add parallel execution support
4. Add iteration limit validation

#### 3.3 Merge Step (1 day)
**Tasks:**
1. Add `WorkflowStepKind::Merge` variant
2. Implement `execute_merge_step()`
3. Support all merge modes

#### 3.4 Switch Step (1 day)
**Tasks:**
1. Add `WorkflowStepKind::Switch` variant
2. Implement `execute_switch_step()`
3. Integrate routing logic into main executor loop

**Dependencies:** Expression language (for items/values expressions)
**Risks:** Medium (complexity, edge cases)

---

## Testing Strategy

### Manual Testing Scenarios

**Grouped Conditions:**
```json
{
  "workflow": "complex_conditions",
  "steps": [
    {
      "id": "check_all",
      "kind": "condition",
      "config": {
        "condition_group": {
          "operator": "all",
          "conditions": [
            { "path": "$status", "operator": "equals", "value": "active" },
            { "path": "$priority", "operator": "greater_than", "value": 5 }
          ]
        }
      },
      "next": "process"
    },
    {
      "id": "check_any",
      "kind": "condition",
      "config": {
        "condition_group": {
          "operator": "any",
          "conditions": [
            { "path": "$status", "operator": "equals", "value": "urgent" },
            { "path": "$priority", "operator": "greater_than", "value": 10 }
          ]
        }
      },
      "next": "escalate"
    }
  ]
}
```

**Expression Transformations:**
```json
{
  "step": "format_names",
  "kind": "tool",
  "tool": "filesystem",
  "args": {
    "path": "$items.map(x => x.name).upper().join('\n')"
  }
}
```

**Loop over items:**
```json
{
  "id": "process_items",
  "kind": "loop",
  "config": {
    "items": "$step.response.data",
    "item_variable": "item",
    "body_step": "process_single"
  }
}
```

### Edge Case Tests

**Empty arrays:**
- Loop over `[]` should return empty results
- `sum([])` should return `0`
- `avg([])` should error

**Null values:**
- `$missing.path` should return `null`
- `$items.filter(x => x.missing)` should return `[]`
- `null == null` should be `true`

**Deep nesting:**
- Condition groups at depth 6 should fail validation
- Expression chains at depth 11 should fail parsing

**Circular references:**
- Workflow step self-reference should fail loading
- Mutual workflow recursion should detect at runtime

### Performance Tests

- Loop with 1000 items (should complete in < 5s)
- Expression with 10,000 element array (should not OOM)
- 50-level nested conditions (should fail quickly with error)

---

## File Structure

### Modified Files

```
rustic-ai-core/src/workflows/
├── types.rs              # Add new types (ConditionGroup, etc.)
├── executor.rs            # Implement new logic
├── loader.rs              # Add validation
├── expressions.rs        # NEW: Expression parser/evaluator
└── mod.rs                # Export new module

docs/
└── config.schema.json     # Add workflow config schemas
```

### New Types Summary

**types.rs additions:**
- `LogicalOperator` enum
- `Condition` struct
- `ConditionGroup` struct
- `WorkflowStepKind` variants: `Wait`, `Loop`, `Merge`, `Switch`

**expressions.rs new module:**
- `Expression` enum
- `BinaryOperator` enum
- `Path` struct
- `ExpressionEvaluator` struct
- Parser functions
- Function library

---

## Success Metrics

### Functional
- [ ] Grouped conditions with nested groups work correctly
- [ ] Expression language supports 15+ functions
- [ ] Wait/delay step executes correctly
- [ ] Loop step processes arrays sequentially
- [ ] Loop step supports parallel execution
- [ ] Merge step combines data correctly
- [ ] Switch step routes to correct branch

### Quality
- [ ] All edge cases handled gracefully
- [ ] Error messages are clear and actionable
- [ ] Performance comparable to current implementation
- [ ] Backward compatible with existing workflows

### Documentation
- [ ] config.schema.json updated
- [ ] Example workflows provided
- [ ] CLI help updated for new step types

---

## Appendix: Example Workflows

### Example 1: Complex Approval Flow

```yaml
name: "approval_workflow"
version: "1.0.0"

steps:
  - id: "check_approval"
    kind: "condition"
    config:
      condition_group:
        operator: "all"
        conditions:
          - path: "$request.amount"
            operator: "less_than_or_equal"
            value: 1000
          - path: "$request.department"
            operator: "equals"
            value: "sales"
    on_success: "auto_approve"
    on_failure: "manual_review"

  - id: "auto_approve"
    kind: "tool"
    tool: "http"
    args:
      url: "https://api.example.com/approve"
      method: "POST"
      body: "$request"

  - id: "manual_review"
    kind: "wait"
    config:
      until_expression: "$review.status == 'approved'"
      timeout_seconds: 86400
```

### Example 2: Batch Processing with Loop

```yaml
name: "batch_processing"
version: "1.0.0"

steps:
  - id: "fetch_items"
    kind: "tool"
    tool: "http"
    outputs:
      items: "$response.data"

  - id: "process_items"
    kind: "loop"
    config:
      items: "$fetch_items.items"
      item_variable: "item"
      body_step: "process_single"

  - id: "process_single"
    kind: "agent"
    agent: "processor"
    args:
      data: "$loop.item"

  - id: "merge_results"
    kind: "merge"
    config:
      mode: "append"
      inputs:
        results: "$process_items.results"
```

### Example 3: Multi-Branch Routing

```yaml
name: "status_router"
version: "1.0.0"

steps:
  - id: "check_status"
    kind: "switch"
    config:
      value: "$input.status"
      cases:
        - value: "success"
          next: "handle_success"
        - value: "error"
          next: "handle_error"
        - pattern: "^retry"
          next: "handle_retry"
      default: "handle_unknown"

  - id: "handle_success"
    kind: "tool"
    tool: "filesystem"
    args:
      path: "/data/success.log"
      content: "$workflow.start_time: Success"

  - id: "handle_error"
    kind: "tool"
    tool: "filesystem"
    args:
      path: "/data/error.log"
      content: "$workflow.start_time: $input.error"

  - id: "handle_retry"
    kind: "wait"
    config:
      duration_seconds: 60
    next: "retry_request"
```

---

**End of Plan**
