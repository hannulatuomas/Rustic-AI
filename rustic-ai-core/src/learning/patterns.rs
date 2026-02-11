use super::types::MistakeType;

pub fn classify_tool_failure(tool: &str, exit_code: Option<i32>, output: &str) -> MistakeType {
    let normalized_tool = tool.to_ascii_lowercase();
    let normalized_output = output.to_ascii_lowercase();

    if normalized_output.contains("permission denied") {
        return MistakeType::PermissionDenied;
    }
    if normalized_output.contains("timed out") || normalized_output.contains("timeout") {
        return MistakeType::ToolTimeout;
    }
    if normalized_output.contains("no such file") || normalized_output.contains("not found") {
        return MistakeType::FileNotFound;
    }
    if normalized_output.contains("test failed") || normalized_output.contains("assertion failed") {
        return MistakeType::TestFailure;
    }
    if normalized_output.contains("compile error")
        || normalized_output.contains("compilation failed")
    {
        return MistakeType::CompilationError;
    }

    if normalized_tool == "filesystem" && matches!(exit_code, Some(2)) {
        return MistakeType::FileNotFound;
    }
    if normalized_tool == "shell" && matches!(exit_code, Some(124 | 137 | 143)) {
        return MistakeType::ToolTimeout;
    }

    MistakeType::WrongApproach
}

pub fn classify_error_message(message: &str) -> MistakeType {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("permission denied") {
        MistakeType::PermissionDenied
    } else if normalized.contains("timed out") || normalized.contains("timeout") {
        MistakeType::ToolTimeout
    } else if normalized.contains("not found") || normalized.contains("no such file") {
        MistakeType::FileNotFound
    } else if normalized.contains("test failed") || normalized.contains("assertion") {
        MistakeType::TestFailure
    } else if normalized.contains("compile") || normalized.contains("compilation") {
        MistakeType::CompilationError
    } else {
        MistakeType::WrongApproach
    }
}

pub fn suggest_fix(mistake_type: MistakeType) -> Option<String> {
    let suggestion = match mistake_type {
        MistakeType::PermissionDenied => {
            "Check permission policy or adjust file/system access scope."
        }
        MistakeType::ToolTimeout => {
            "Reduce task scope, increase timeout, or switch to a lighter command."
        }
        MistakeType::FileNotFound => {
            "Verify the path exists and re-run discovery before mutation steps."
        }
        MistakeType::CompilationError => {
            "Run the compiler with full diagnostics and fix syntax/type errors first."
        }
        MistakeType::TestFailure => {
            "Run only failing tests with verbose output, then fix root causes."
        }
        MistakeType::WrongApproach => {
            "Try a different tool or break the task into smaller deterministic steps."
        }
    };

    Some(suggestion.to_owned())
}
