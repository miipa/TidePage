import sqlite3, os, time
db = r"C:\Users\A\AppData\Roaming\com.a.flowread\flowread.db"
wal = db + "-wal"
print("time =", time.strftime("%Y-%m-%d %H:%M:%S"))
print("db bytes =", os.path.getsize(db), " wal bytes =", os.path.getsize(wal) if os.path.exists(wal) else 0)
con = sqlite3.connect(db, timeout=15)
cur = con.cursor()
def q(label, sql, many=True):
    print("---", label, "---")
    try:
        rows = cur.execute(sql).fetchall()
        print(rows)
    except Exception as e:
        print("ERR", type(e).__name__, e)
q("group", "SELECT meta_done, COUNT(*) FROM books GROUP BY meta_done")
q("wal_checkpoint(TRUNCATE)", "PRAGMA wal_checkpoint(TRUNCATE)")
q("journal_mode", "PRAGMA journal_mode")
q("meta_done=0 count", "SELECT COUNT(*) FROM books WHERE meta_done = 0")
# schema of books meta columns
try:
    cols = [r[1] for r in cur.execute("PRAGMA table_info(books)").fetchall()]
    print("books columns:", cols)
except Exception as e:
    print("ERR schema", e)
# any other tx-related tables
try:
    print("tables:", [r[0] for r in cur.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()])
except Exception as e:
    print("ERR tables", e)
con.close()
print("after checkpoint wal bytes =", os.path.getsize(wal) if os.path.exists(wal) else 0)
