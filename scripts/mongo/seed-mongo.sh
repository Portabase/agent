#!/usr/bin/env bash

# Usage:
# ./seed-mongo.sh           -> no auth
# ./seed-mongo.sh auth      -> with auth

MODE=${1:-noauth}  # default: noauth

# Default values (can be overridden via .env)
MONGO_DB=${MONGO_DB:-testdb}
MONGO_PORT=${MONGO_PORT:-27017}

# Auth variables
MONGO_AUTH_USERNAME=${MONGO_AUTH_USERNAME:-root}
MONGO_AUTH_PASSWORD=${MONGO_AUTH_PASSWORD:-rootpassword}
MONGO_AUTH_DB=${MONGO_AUTH_DB:-testdbauth}

NUM_USERS=${NUM_USERS:-5000}

# Build connection string
if [ "$MODE" = "auth" ]; then
    MONGO_CONTAINER=${MONGO_AUTH_CONTAINER:-db-mongodb-auth}
    URI="mongodb://$MONGO_AUTH_USERNAME:$MONGO_AUTH_PASSWORD@localhost:$MONGO_PORT/$MONGO_AUTH_DB?authSource=admin"
    echo "Using authenticated MongoDB..."
else
    MONGO_CONTAINER=${MONGO_CONTAINER:-db-mongodb}
    URI="mongodb://localhost:$MONGO_PORT/$MONGO_DB"
    echo "Using non-authenticated MongoDB..."
fi

# Create temp JS seed file
SEED_FILE=$(mktemp /tmp/seed.XXXX.js)

cat <<EOF > "$SEED_FILE"
db = connect("$URI");

// Drop collections if exist
if (db.users) db.users.drop();
if (db.products) db.products.drop();

// Generate users
const users = [];
for (let i = 1; i <= $NUM_USERS; i++) {
    users.push({
        username: "user" + i,
        email: "user" + i + "@example.com",
        age: Math.floor(Math.random() * 60) + 18,
        active: Math.random() > 0.5,
        createdAt: new Date()
    });
}
db.users.insertMany(users);

// Generate products
const products = [];
for (let i = 1; i <= Math.floor($NUM_USERS/10); i++) {
    products.push({
        name: "Product " + i,
        price: parseFloat((Math.random()*100).toFixed(2)),
        stock: Math.floor(Math.random()*500),
        createdAt: new Date()
    });
}
db.products.insertMany(products);

print("Seed complete: " + users.length + " users, " + products.length + " products.");
EOF

docker cp "$SEED_FILE" "$MONGO_CONTAINER:/seed.js"
docker exec -it "$MONGO_CONTAINER" mongosh /seed.js
rm "$SEED_FILE"
