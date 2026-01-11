# lighthouse.jq - Extract scores from Lighthouse JSON report
#
# Usage: vecq lighthouse-report.json -q 'lighthouse_badges'
#
# Input: Google Lighthouse JSON output
# Output: A formatted string of scores (Performance | Accessibility | ...)

# ------------------------------------------------------------------
# Helpers
# ------------------------------------------------------------------

# Convert 0-1 score to Percentage Integer (0.98 -> 98)
def _to_pct:
  (. * 100) | round;

# Choose an icon based on score threshold
def _score_icon:
  if . >= 90 then "🟢"
  elif . >= 50 then "jb🟠"
  else "🔴"
  end;

# Format a single category object
# Input: { "score": 0.98, "title": "Performance" ... }
def _format_category:
  (.score | _to_pct) as $pct |
  ($pct | _score_icon) + " **" + .title + "**: " + ($pct | tostring);

# ------------------------------------------------------------------
# Main Functions
# ------------------------------------------------------------------

def lighthouse_badges:
  # Lighthouse stores scores in the top-level `.categories` object
  # varying by keys: performance, accessibility, best-practices, seo, pwa
  
  [
    (.categories.performance | _format_category),
    (.categories.accessibility | _format_category),
    (.categories."best-practices" | _format_category),
    (.categories.seo | _format_category)
  ] 
  # Join all badges with a pipe separator
  | join(" | ")
  ;

# Alternative: Detailed Markdown Table
def lighthouse_table:
  "| Category | Score | Status |\n" +
  "| :--- | :--- | :--- |\n" +
  (
    .categories | to_entries | map(.value) |
    "| " + .title + " | " + (.score | _to_pct | tostring) + " | " + (.score | _to_pct | _score_icon) + " |"
  )
  ;
