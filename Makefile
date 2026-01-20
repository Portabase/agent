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


seed-postgres:
	@echo "Seeding Postgres..."
	docker exec -i -e PGPASSWORD=$$PG_PASSWORD $$PG_CONTAINER \
		psql -U $$PG_USER -d $$PG_DB < ./scripts/postgres/seed.sql

seed-postgres-1gb:
	@echo "Seeding Postgres..."
	docker exec -i -e PGPASSWORD=$$PG_PASSWORD $$PG_CONTAINER \
		psql -U $$PG_USER -d $$PG_DB < ./scripts/postgres/seed-1gb.sql

seed-all: seed-mongo seed-mysql seed-postgres seed-postgres-1gb
