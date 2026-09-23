-- Synthetic ZCode fixture. Physical order differs from canonical sequence.
CREATE TABLE session(id TEXT PRIMARY KEY,directory TEXT,title TEXT,time_created INTEGER,time_updated INTEGER,parent_id TEXT,time_archived INTEGER);
CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT,sequence INTEGER);
CREATE TABLE part(id TEXT PRIMARY KEY,message_id TEXT,session_id TEXT,time_created INTEGER,data TEXT,sequence INTEGER);
INSERT INTO session VALUES('same','/fixture','ZCode fixture',1000000,1002000,NULL,NULL);
INSERT INTO message VALUES('a','same',1000000,'{"role":"assistant"}',1);
INSERT INTO message VALUES('u','same',1001000,'{"role":"user"}',0);
INSERT INTO part VALUES('pa','a','same',1000000,'{"type":"text","text":"zcode needle answer"}',0);
INSERT INTO part VALUES('pu','u','same',1001000,'{"type":"text","text":"zcode needle question"}',0);
