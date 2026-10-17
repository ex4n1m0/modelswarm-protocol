-- Rollback for migration 0007 (D6 pairing). Consumed accounting is
-- pure evidence — dropping the table removes no state any other
-- surface depends on (the balance is derived at read time).
DROP TABLE IF EXISTS consumed_accounting;
