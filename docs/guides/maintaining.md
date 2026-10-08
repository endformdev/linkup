# Updating and Uninstalling

## Updating

If you installed Linkup via the install script, use the built-in update command:

```sh
linkup update
```

This stops any running Linkup session, downloads the newest version supported by
your Linkup worker, swaps it in place, and reports when it's done. On Linux, it
also re-applies the `cap_net_bind_service` capability needed to bind to ports
80/443.

### Worker compatibility

The CLI only updates to versions with the same major version as your deployed
worker. If a newer major version is out, Linkup tells you it's available once
the worker is updated. Whoever manages your Linkup deployment can update the
worker by running [`linkup infra deploy`](deploy-linkup.md) with the new CLI.

To update to the newest version regardless of the worker, for example to get
the CLI you need to deploy the new worker, pass `--force`:

```sh
linkup update --force
```

Workers on 4.1.1 or older don't report their version, so the CLI treats them as
4.1.1.

To update to (or stay on) the pre-release channel, pass `--channel beta`:

```sh
linkup update --channel beta
```

To go back to stable:

```sh
linkup update --channel stable
```

The CLI caches the latest known releases and your worker's version to avoid
hitting the network on every command. To bypass that cache, pass `--skip-cache`:

```sh
linkup update --skip-cache
```

---

## Uninstalling

```sh
linkup uninstall
```

You will be asked to confirm before anything is removed. On confirmation, it:

1. Stops any running Linkup session (`linkup stop`)
2. Uninstalls Local DNS if it was installed (`linkup local-dns uninstall`)
3. Removes the Linkup binary, using the right method for how you installed it
   (Cargo or install script)
4. Removes the `~/.linkup/` directory and all state, certificates, and logs
   stored there

---

← [Local DNS](local-dns.md) · [Docs home](../README.md) · [Troubleshooting](troubleshooting.md) →
