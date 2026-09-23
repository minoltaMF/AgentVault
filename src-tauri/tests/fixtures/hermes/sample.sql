-- Synthetic Hermes fixture. Optional model_config/reasoning columns intentionally absent.
CREATE TABLE sessions(id TEXT PRIMARY KEY, source TEXT, model TEXT, started_at REAL, ended_at REAL, title TEXT);
CREATE TABLE messages(id INTEGER PRIMARY KEY,session_id TEXT,role TEXT,content TEXT,timestamp REAL,tool_calls TEXT,tool_call_id TEXT);
INSERT INTO sessions VALUES('same','cli','fixture-model',1000,NULL,'Hermes fixture');
INSERT INTO messages VALUES(1,'same','user','hermes needle question',1001,NULL,NULL);
INSERT INTO messages VALUES(2,'same','assistant','hermes needle answer',1002,NULL,NULL);
INSERT INTO messages VALUES(3,'same','tool','not conversation',1003,NULL,'call-1');
