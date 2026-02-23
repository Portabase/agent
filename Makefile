include .env
export $(shell sed 's/=.*//' .env)

.PHONY: seed-mongo seed-mysql seed-postgres

seed-mongo:
	@echo "Seeding MongoDB..."
	bash ./scripts/mongo/seed-mongo.sh

seed-mongo-auth:
	@echo "Seeding MongoDB with auth..."
	bash ./scripts/mongo/seed-mongo.sh auth

seed-mysql:
	@echo "Seeding MySQL..."
	mysql -h 127.0.0.1 -P "$$MYSQL_PORT" -u "$$MYSQL_USER" -p"$$MYSQL_PASSWORD" "$$MYSQL_DB" < ./scripts/mysql/seed-mysql.sql

seed-mysql-1gb:
	@echo "Seeding MySQL..."
	mysql -h 127.0.0.1 -P "$$MYSQL_PORT" -u "$$MYSQL_USER" -p"$$MYSQL_PASSWORD" "$$MYSQL_DB" < ./scripts/mysql/seed-1gb.sql


seed-postgres:
	@echo "Seeding Postgres..."
	docker exec -i -e PGPASSWORD=$$PG_PASSWORD $$PG_CONTAINER \
		psql -U $$PG_USER -d $$PG_DB < ./scripts/postgres/seed.sql

seed-postgres-1gb:
	@echo "Seeding Postgres..."
	docker exec -i -e PGPASSWORD=$$PG_PASSWORD $$PG_CONTAINER \
		psql -U $$PG_USER -d $$PG_DB < ./scripts/postgres/seed-1gb.sql


SQLITE_SEED_FILE := $(if $(filter big,$(SEED)),./scripts/sqlite/seed-big.sql,./scripts/sqlite/seed.sql)

seed-sqlite:
	@echo "Seeding Sqlite..."
	@echo "Run as root to fix permissions inside the volume"
	docker exec -u 0 -it db-sqlite sh -c "chmod -R 777 /workspace/data"
	@echo "Create the database file (if it doesn’t exist)"
	docker exec -u 0 -it db-sqlite sh -c "touch /workspace/data/app.db"
	@echo "Seed the database"
	docker exec -i db-sqlite sh -c "sqlite3 /workspace/data/app.db" < $(SQLITE_SEED_FILE)
	@echo "Verify"
	docker exec -it db-sqlite sqlite3 /workspace/data/app.db "SELECT name FROM sqlite_master WHERE type='table';"
	@echo "Done"



seed-all: seed-mongo seed-mysql seed-postgres seed-postgres-1gb
