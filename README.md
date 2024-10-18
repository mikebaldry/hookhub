# Hookhub

A small rust app to accept incoming web requests (assuming webhooks) and feed the requests back to all connected clients. The client then forwards the request to a local serevr. 

The motivation for this is when a team is working on a project that uses an external service which sends webhooks and every developer needs to be able to receive the webhooks. It is quite inconvenient to set up multiple webhooks and adding or removing them and setting up reverse proxies like ngrok and their associated cost.

It's very simple at the moment - static response to any request of 200 and forwards it to all clients connected via websockets. 

## Running the server

By default this will listen on localhost:9873 and have a secret of abc123. 

These are the only 2 configuration options and are configurable by setting the env variables `HOOKHUB_SECRET` and `HOOKHUB_BIND_ADDR`.

## Running the client

There are 3 configuration options that can either passed on the command line or set in env variables.
- `--remote` / `HOOKHUB_REMOTE` - The Hookhub server to connect to, e.g. wss://where-its-running-and-receiving-hook-calls.com/
- `--secret` / `HOOKHUB_SECRET` - The secret set up on the server for simple authentication
- `--local` / `HOOKHUB_LOCAL` - The local web server to forward incoming requests to when received from the remote

