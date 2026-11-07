# Seed instructions


```bash
make seed-mongo     
make seed-mongo-auth  
make seed-mysql      
make seed-mysql-1gb    
make seed-postgres      
make seed-postgres-1gb  
make seed-all   
make seed-sqlite   
make seed-sqlite SEED=big   
just seed-files-1gb
```

## MSSQL backup formats

```bash
just seed-mssql
```

`databases.json` has one entry per format, all on `myappdb`. The `bak` entries use the folder `/var/opt/mssql/backup`, which the seed creates and `rust-dev` sees at `/mssql-shared/backup`. If your container is older than that mount, run `docker compose up -d rust-app`.

| Entry | Options | Backup file |
|---|---|---|
| Test database 13 - MsSQL | none | `.dacpac` |
| Test database 15 - MsSQL BACPAC | `backup_format: bacpac` | `.bacpac` |
| Test database 16 - MsSQL BAK | `backup_format: bak` | `.bak` |
| Test database 17 - MsSQL BAK Verify | `backup_format: bak`, `bak_verify: true` | `.bak`, with `RESTORE VERIFYONLY` in the job log |

## Verify commands 

### Sqlite

```bash
docker exec -it db-sqlite sqlite3 /workspace/data/app.db "SELECT * FROM users LIMIT 10;"  
```

```bash
docker exec -it db-sqlite sqlite3 /workspace/data/app.db "SELECT name FROM sqlite_master WHERE type='table';"
```

```bash
docker compose -f docker-compose.test.yml run agent-test bash -c "cargo clean && cargo test" 
```