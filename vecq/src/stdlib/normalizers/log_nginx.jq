# Helper: Convert 3-letter month to number
def month_to_num:
  if . == "Jan" then "01"
  elif . == "Feb" then "02"
  elif . == "Mar" then "03"
  elif . == "Apr" then "04"
  elif . == "May" then "05"
  elif . == "Jun" then "06"
  elif . == "Jul" then "07"
  elif . == "Aug" then "08"
  elif . == "Sep" then "09"
  elif . == "Oct" then "10"
  elif . == "Nov" then "11"
  elif . == "Dec" then "12"
  else "01" end;

# Helper: Parse NGINX time "13/Jan/2026:21:29:00 +0000" -> ISO8601
def parse_nginx_time:
  # Split: ["13", "Jan", "2026:21:29:00 +0000"]
  split("/") as $p1 |
  $p1[0] as $d |
  ($p1[1] | month_to_num) as $m |
  
  # Split rest: "2026:21:29:00 +0000" -> ["2026", "21", "29", "00 +0000"]
  ($p1[2] | split(":")) as $p2 |
  $p2[0] as $y |
  $p2[1] as $H |
  $p2[2] as $M |
  
  # Split last: "00 +0000" -> ["00", "+0000"]
  ($p2[3] | split(" ")) as $p3 |
  $p3[0] as $S |
  
  # Construct ISO8601: YYYY-MM-DDTHH:MM:SSZ
  ($y + "-" + $m + "-" + $d + "T" + $H + ":" + $M + ":" + $S + "Z");

def nginx_to_log:
  # Regex: ^(?<remote_addr>\S+) \S+ (?<remote_user>\S+) \[(?<time_local>.*?)\] "(?<request_method>\S+) (?<request_uri>\S+) (?<server_protocol>\S+)" (?<status>\d+) (?<body_bytes_sent>\d+) "(?<http_referer>.*?)" "(?<http_user_agent>.*?)"$
  re_capture("^(?<remote_addr>\\S+) \\S+ (?<remote_user>\\S+) \\[(?<time_local>.*?)\\] \"(?<request_method>\\S+) (?<request_uri>\\S+) (?<server_protocol>\\S+)\" (?<status>\\d+) (?<body_bytes_sent>\\d+) \"(?<http_referer>.*?)\" \"(?<http_user_agent>.*?)\"$") |
  {
    timestamp: (try (.time_local | parse_nginx_time | fromdateiso8601 | todate) catch (now | todate)),
    level: (if (.status | tonumber) >= 500 then "ERROR" elif (.status | tonumber) >= 400 then "WARN" else "INFO" end),
    message: (.request_method + " " + .request_uri + " " + .status),
    source: "nginx",
    data: {
      remote_addr: .remote_addr,
      remote_user: .remote_user,
      body_bytes: (.body_bytes_sent | tonumber),
      referer: .http_referer,
      user_agent: .http_user_agent,
      raw_time: .time_local
    }
  };
