# Linkup documentation

> Run the services you change, get the rest for free

Linkup lets you combine local and remote web services to create low-cost yet complete development environments.

Linkup consists of:

- A proxy deployed as a Cloudflare worker for traffic routing
- A local CLI to manage sessions

## Getting started

- [What is Linkup?](explanation/what-is-linkup.md): new to Linkup? Start here to understand what it does and why it exists.
- [Deploy Linkup to Cloudflare](guides/deploy-linkup.md): set up the Cloudflare infrastructure required to run Linkup sessions.
- [Run a Local Session](guides/local-env.md): install the CLI and run your first local Linkup session.

## Sessions

- [Managing Sessions](guides/sessions.md): learn about tunneled and preview session types, and how to manage them.
- [Preview Environments](guides/preview-env.md): create fully remote sessions from deployed services. No local machine required.

## Guides

- [Configure Linkup](guides/configure.md): describe the layout of your services in a Linkup config file.
- [Local DNS](guides/local-dns.md): speed up your local session by resolving domains directly on your machine.
- [Updating and Uninstalling](guides/maintaining.md): keep Linkup up to date and remove it if needed.
- [Troubleshooting](guides/troubleshooting.md): solutions to common problems with tunnels, configuration, and more.

## Concepts

- [What does a setup look like?](explanation/what-does-a-setup-look-like.md): what it could look like to run a full Linkup deployment.
- [How Linkup works](explanation/how-it-works.md): how to configure services to work with Linkup.

## Reference

- [Config Reference](reference/config.md): every field accepted by the Linkup configuration file.
- [Shell Completion](reference/shell-completion.md): generate shell autocompletions for the `linkup` CLI.
- [Cloudflare Resources](reference/cloudflare-resources.md): the resources Linkup needs in Cloudflare.
