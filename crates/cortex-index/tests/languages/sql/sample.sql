-- The shop's schema, with each statement the SQL query reads and two it does not.
CREATE TABLE users (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS public.orders (
  id INTEGER PRIMARY KEY,
  user_id INTEGER REFERENCES users (id)
);

CREATE VIEW active_users AS
  SELECT * FROM users;

CREATE MATERIALIZED VIEW order_counts AS
  SELECT user_id, count(*) FROM public.orders GROUP BY user_id;

CREATE TYPE mood AS ENUM ('happy', 'sad');

CREATE FUNCTION add(a integer, b integer) RETURNS integer
  AS $$ SELECT a + b $$
  LANGUAGE sql;

CREATE OR REPLACE FUNCTION touch() RETURNS trigger AS $$
BEGIN
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE INDEX users_name ON users (name);

INSERT INTO users (name) VALUES ('ada');
