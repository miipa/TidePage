import sqlite3, os, sys, time, glob

appdata = os.environ["APPDATA"]
db = os.path.join(appdata, "com.a.flowread", "flowread.db")
print("db:", db, "exists:", os.path.exists(db))
for f in ["flowread.db", "flowread.db-wal", "flowread.db-shm"]:
    p = os.path.join(appdata, "com.a.flowread", f)
    if os.path.exists(p):
        print(f"{f}: {os.path.getsize(p):,} bytes")

con = sqlite3.connect(db, timeout=30)
cur = con.cursor()

def q(sql):
    try:
        cur.execute(sql)
        return cur.fetchall()
    except Exception as e:
        return [("ERR", str(e))]

print("\ntime", time.strftime("%Y-%m-%d %H:%M:%S"))
print("meta_done groups:", q("SELECT meta_done, COUNT(*) FROM books GROUP BY meta_done"))
print("wal_checkpoint(TRUNCATE):", q("PRAGMA wal_checkpoint(TRUNCATE)"))
print("journal_mode:", q("PRAGMA journal_mode"))
print("meta_done=0 count:", q("SELECT COUNT(*) FROM books WHERE meta_done = 0"))
print("books total:", q("SELECT COUNT(*) FROM books"))
con.close()
