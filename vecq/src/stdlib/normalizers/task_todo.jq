def todo_to_task:
  # Input: "TODO: Fix the bug priority:high"
  # Output: Task Schema
  # Regex: TODO:\s*(?<title>.*?)\s*(?:priority:(?<priority>\w+))?$
  
  # Try to capture with regex
  re_capture("TODO:\\s*(?<title>.*?)\\s*(?:priority:(?<priority>\\w+))?$") |
  {
    element_type: "task",
    id: ("task_" + (now | tostring)), # Generate ID based on timestamp
    title: .title,
    status: "open",
    priority: (.priority // "medium"),
    assignee: null,
    tags: ["todo"],
    metadata: {
      source: "todo_comment",
      created: (now | todate)
    }
  };
