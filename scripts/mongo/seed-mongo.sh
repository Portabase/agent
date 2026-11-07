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

function seed(name, count, build) {
    const collection = db.getCollection(name);
    collection.drop();
    const documents = [];
    for (let i = 1; i <= count; i++) documents.push(build(i));
    collection.insertMany(documents);
    return documents.length;
}

const noise = (words) => Array.from({ length: words }, () => Math.random().toString(36).slice(2)).join("");

const users = seed("users", $NUM_USERS, (i) => ({
    username: "user" + i,
    email: "user" + i + "@example.com",
    age: Math.floor(Math.random() * 60) + 18,
    active: Math.random() > 0.5,
    createdAt: new Date()
}));

const products = seed("products", Math.floor($NUM_USERS/10), (i) => ({
    name: "Product " + i,
    price: parseFloat((Math.random()*100).toFixed(2)),
    stock: Math.floor(Math.random()*500),
    createdAt: new Date()
}));

const excludable = {
    events: seed("events", $NUM_USERS*4, (i) => ({ user: "user" + (i % $NUM_USERS + 1), type: "page_view", payload: noise(40), createdAt: new Date() })),
    sessions: seed("sessions", $NUM_USERS*2, (i) => ({ user: "user" + (i % $NUM_USERS + 1), token: noise(8), expiresAt: new Date(Date.now() + 3600000) })),
    cache_pages: seed("cache_pages", $NUM_USERS, (i) => ({ key: "/page/" + i, html: noise(80) })),
    cache_api: seed("cache_api", Math.floor($NUM_USERS/2), (i) => ({ key: "/api/" + i, body: noise(40) })),
    tmp_import: seed("tmp_import", Math.floor($NUM_USERS/2), (i) => ({ row: i, raw: noise(20) }))
};

print("Seed complete: " + users + " users, " + products + " products.");
print("Collections for exclude_collections / exclude_collections_with_prefix: " + JSON.stringify(excludable));
EOF

docker cp "$SEED_FILE" "$MONGO_CONTAINER:/seed.js"
docker exec -it "$MONGO_CONTAINER" mongosh /seed.js
rm "$SEED_FILE"
