# Heuristically detect and normalize input data
def auto_normalize:
  if type == "array" then map(auto_normalize)
  elif type == "object" then
    if has("remote_addr") and has("request") then nginx_to_log
    elif has("__REALTIME_TIMESTAMP") then journald_to_log
    elif has("number") and has("title") and has("state") then github_to_task
    elif has("reason") and .reason == "compiler-message" then cargo_to_artifact
    elif has("x-source") and .["x-source"] == "openwebui" then openwebui_to_chat
    else .
    end
  else .
  end;
