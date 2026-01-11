# npm_audit.jq - Transform npm audit JSON to a Security Report
#
# Usage: npm audit --json | vecq -q 'audit_summary'
#
# Input: Standard `npm audit --json` output
# Output: Markdown table of high/critical vulnerabilities

# ------------------------------------------------------------------
# Helpers
# ------------------------------------------------------------------

# Emoji mapping for severity levels
def _severity_icon:
  if . == "critical" then "🔴"
  elif . == "high" then "qh🟠"
  elif . == "moderate" then "🟡"
  else "⚪"
  end;

# ------------------------------------------------------------------
# Main Functions
# ------------------------------------------------------------------

# Main entry point: summaries the audit report
def audit_summary:
  # Title and Header
  "# 🛡️ NPM Security Report\n\n" +
  
  # Summary Statistics (if available in metadata)
  (if .metadata.vulnerabilities then
    "**Summary**: " + 
    "Critical: " + (.metadata.vulnerabilities.critical | tostring) + " | " +
    "High: " + (.metadata.vulnerabilities.high | tostring) + "\n\n"
  else "" end) +

  # Table Header
  "| Severity | Package | Title | Fix |\n" +
  "| :--- | :--- | :--- | :--- |\n" +

  # Table Rows: Iterate over advisories (legacy) or vulnerabilities (modern)
  # using `to_entries` because modern v2 format uses package names as keys
  ((.vulnerabilities // {}) | to_entries | map(.value) | 
   
   # Filter: We only care about High and Critical issues
   select(.severity == "high" or .severity == "critical") |
   
   # Format: Construct the table row
   "| " + (.severity | _severity_icon) + " " + .severity + 
   " | `" + .name + "`" +
   " | " + .via[0].title + # Grab the first advisory titles
   " | `" + (if .fixAvailable == true then "npm audit fix" else "Manual Review" end) + "`" +
   " |\n"
  )
  ;
