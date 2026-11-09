<br />
<div align="center">
  <a href="https://portabase.io">
    <img src="/.github/assets/logo.png" alt="Logo" width="80" height="80">
  </a>

<h3 align="center">Portabase Agent (Rust)</h3>

  <p align="center" style="margin-top: 20px; font-style: italic;">
    <i>Portabase Agent is use with Portabase Server. This service is for backup/restore db instances.
    <a href="https://github.com/Portabase/portabase">Portabase Server</a></i>
  </p>

[![License: Apache](https://img.shields.io/badge/License-apache-yellow.svg)](LICENSE)
[![Docker Pulls](https://img.shields.io/docker/pulls/portabase/agent?color=brightgreen)](https://hub.docker.com/r/portabase/agent)
[![Helm Chart](https://img.shields.io/badge/Helm-Kubernetes-326ce5?logo=helm&logoColor=white)](https://github.com/Portabase/agent/pkgs/container/charts%2Fportabase-agent)
[![Platform](https://img.shields.io/badge/platform-linux%20%7C%20macos%20%7C%20windows-lightgrey)](https://github.com/Portabase/portabase)
[![Support Portabase](https://img.shields.io/badge/Support-Portabase-orange)](https://www.buymeacoffee.com/portabase)

[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-336791?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![MySQL](https://img.shields.io/badge/MySQL-4479A1?logo=mysql&logoColor=white)](https://www.mysql.com/)
[![SQLite](https://img.shields.io/badge/-SQLite-blue?logo=sqlite&logoColor=white)](https://sqlite.org/)
[![Redis](https://img.shields.io/badge/Redis-DC382D?style=flat&logo=Redis&logoColor=white)](https://redis.io/)
[![MariaDB](https://img.shields.io/badge/MariaDB-003545?logo=mariadb&logoColor=white)](https://mariadb.org/)
[![MongoDB](https://img.shields.io/badge/-MongoDB-13aa52?logo=mongodb&logoColor=white)](https://www.mongodb.com/)
[![Valkey](https://img.shields.io/badge/Valkey-6284fc?style=flat&logo=Valkey&logoColor=white)](https://valkey.io/)
[![Firebird](https://img.shields.io/badge/Firebird-f55b14?style=flat&logo=Firebird&logoColor=white)](https://firebirdsql.org/)


[![Self Hosted](https://img.shields.io/badge/self--hosted-yes-brightgreen)](https://github.com/Portabase/portabase)
[![Open Source](https://img.shields.io/badge/open%20source-❤️-red)](https://github.com/Portabase/portabase)

[![Rust][Rust]][Rust-url]
[![Tokio][Tokio]][Tokio-url]
[![Redis][Redis]][Redis-url]
[![Docker][Docker]][Docker-url]

  <p>
    <strong>
        <a href="https://portabase.io">Website</a> •
        <a href="https://portabase.io/docs">Documentation</a> •
        <a href="https://www.youtube.com/watch?v=hvLbX5LN1UE">Demo</a> •
        <a href="https://portabase.io/docs/agent/setup">Installation</a> •
        <a href="https://github.com/Portabase/agent-portabase/issues/new?labels=bug&template=bug-report---.md">Report Bug</a> •
        <a href="https://github.com/Portabase/agent-portabase/issues/new?labels=enhancement&template=feature-request---.md">Request Feature</a>
    </strong>
  </p>

</div>

## Installation

You have 4 ways to install Portabase Agent:

- Automated CLI (recommended) - [details](https://portabase.io/docs/agent/setup#cli)
- Docker Compose setup - [details](https://portabase.io/docs/agent/setup#docker)
- Kubernetes with Helm [details](https://portabase.io/docs/agent/setup#helm)
- Development setup - [details](https://portabase.io/docs/agent/setup#development)

**Ensure Docker is installed on your machine before getting started.**

## PostgreSQL physical incremental backups

A `postgresql-cluster` source can use the `physical` method (`options.method = "physical"`, default `archive` = `pg_dumpall`). It takes whole-cluster backups through the replication protocol with server-side zstd compression: a full backup, then incrementals chained to it. `options.full_every_days` (integer >= 1, default 7) starts a new chain once the chain's full backup is that old. A broken chain (failover, restored server, expired WAL summaries) falls back to a full backup automatically.

Server requirements:

- PostgreSQL 17 or later
- A role with `REPLICATION`, and a `pg_hba.conf` entry allowing replication connections from the agent
- `summarize_wal = on`: `ALTER SYSTEM SET summarize_wal = on; SELECT pg_reload_conf();`
- No custom tablespaces

Restore rebuilds a ready-to-start data directory under `$RESTORE_PATH/<source id>/<UTC timestamp>/` (`data/` for PostgreSQL 17 and earlier, `<major>/docker/` for 18 and later) with `pg_combinebackup`, then checks it with `pg_verifybackup`. The running server is never touched. The start commands (host and Docker) are printed at the end of the restore job logs.

A physical restore is refused, before anything is downloaded, unless `RESTORE_PATH` is set to an existing, writable directory. Mount a volume there:

```yaml
    volumes:
      - ./restores:/restores
    environment:
      RESTORE_PATH: /restores
```

With Helm, set `volume.restore.enabled=true` with `volume.restore.hostPath` or `volume.restore.existingClaim`.

## Contributors

[![Contributors](https://contrib.rocks/image?repo=Portabase/agent-rust)](https://github.com/Portabase/agent-rust/graphs/contributors)

## Become a Sponsor

🙏 Thank you to the incredible sponsors for supporting this project! Your contributions help keep Portabase running and growing. If you'd like to join and become a sponsor, please visit the [sponsorship page](https://github.com/sponsors/Portabase) and be part of something great! 🚀

### 💎 Platinum Sponsors

<table>
  <tr>
    <td align="center">
      <a href="https://github.com/sponsors/Portabase">
        <img src="/.github/assets/plus-circle.svg" height="180" alt="Become a platinum sponsor"/>
        <br />
        Become a platinum sponsor
      </a>
    </td>
  </tr>
</table>

### 🥇 Gold Sponsors

<table>
  <tr>
    <td align="center">
      <a href="https://github.com/sponsors/Portabase">
        <img src="/.github/assets/plus-circle.svg" height="150" alt="Become a gold sponsor"/>
        <br />
        Become a gold sponsor
      </a>
    </td>
  </tr>
</table>

### 🥈 Silver Sponsors

<table>
  <tr>
    <td align="center">
      <a href="https://github.com/sponsors/Portabase">
        <img src="/.github/assets/plus-circle.svg" height="100" alt="Become a silver sponsor"/>
        <br />
        Become a silver sponsor
      </a>
    </td>
  </tr>
</table>

### 🤝 Community Sponsors

<table>
  <tr>
    <td align="center">
      <a href="https://github.com/sponsors/Portabase">
        <img src="/.github/assets/plus-circle.svg" height="80" alt="Become a community sponsor"/>
        <br />
        Become a community sponsor
      </a>
    </td>
  </tr>
</table>

## License

Distributed under the Apache License. See `LICENSE.txt` for more information.

[Rust]: https://img.shields.io/badge/rust-000000?style=for-the-badge&logo=rust&logoColor=white

[Tokio]: https://img.shields.io/badge/Tokio-09d3ac?style=for-the-badge&logo=tokio&logoColor=white

[Docker]: https://img.shields.io/badge/Docker-2496ED?logo=docker&logoColor=fff&style=for-the-badge

[Redis]: https://img.shields.io/badge/Redis-DC382D?style=for-the-badge&logo=redis&logoColor=white

[Rust-url]: https://www.rust-lang.org/

[Tokio-url]: https://tokio.rs/

[Redis-url]: https://redis.io/

[Docker-url]: https://www.docker.com/



