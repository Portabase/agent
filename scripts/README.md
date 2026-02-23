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
```

## Verify commands 

### Sqlite

```bash
docker exec -it db-sqlite sqlite3 /workspace/data/app.db "SELECT * FROM users LIMIT 10;"  
```

```bash
docker exec -it db-sqlite sqlite3 /workspace/data/app.db "SELECT name FROM sqlite_master WHERE type='table';"
```
