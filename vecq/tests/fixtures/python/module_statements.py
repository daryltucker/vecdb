# fixture: module-level executable statements and assignments.
#
# Feeds the reachability gate for the Python schema additions
# (`Variable -> variable`, `Module -> module`) and the tier1 module-statement
# test. Everything below is at top level on purpose:
#
# PAGE_SQL      -> module-level Assign   (extract_variables)
# The call to   -> module-level Expr     (module statements, no name to grab)
# rebuild_table -> grows the table
# the __main__  -> the always-present pattern in scripts that agents need to
#                  read without opening handler.py themselves.
#
PAGE_SQL = "DELETE FROM job_queue WHERE status = 'stuck'"

TAB_DELIM = "\t"


def rebuild_table():
    """Grow the ledger before the main block runs."""
    rows = [PAGE_SQL, TAB_DELIM]
    return rows


rebuild_table()

if __name__ == "__main__":
    print(PAGE_SQL)
