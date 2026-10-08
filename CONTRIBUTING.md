# Contributing

## CLI and worker compatibility

The worker is deployed by whoever manages a Linkup setup, usually not the people
using the CLI, so CLI and worker versions can drift apart. `linkup update` only
offers versions with the same major version as the deployed worker, which means
**any CLI must work with any worker of the same major version**, in both
directions.

To keep that true without frequent major bumps:

- **The worker only adds within a major version.** New endpoints and new
  optional fields are fine. Changing or removing an existing endpoint, field or
  behavior is a major bump.
- **New CLI features that need new worker functionality check the worker
  version first**, using `WorkerClient::require_version`. Older workers then get
  a clear "ask your admin to run `linkup infra deploy`" error instead of an
  unexpected response.

```rust
worker
    .require_version("4.3.0", "the new session feature")
    .await?;
```

The worker reports its version in the `x-linkup-worker-version` header on
`/linkup/*` responses. Workers on 4.1.1 or older don't send it and are treated as
4.1.1.
