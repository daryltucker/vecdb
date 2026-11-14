import sqlite3

PAGE_SQL = "SELECT * FROM backend_tracks LIMIT ?"
conn = sqlite3.connect("x.db")
conn.execute("DELETE FROM job_queue WHERE id = ?", (1,))


class Store:
    def save(self, conn: sqlite3.Connection, table: str) -> None:
        conn.execute(f"INSERT INTO {table} (a) VALUES (?)", (1,))


def run(conn: sqlite3.Connection) -> None:
    conn.execute(PAGE_SQL, (10,))
    conn.execute("UPDATE " + "jobs SET x = 1")
