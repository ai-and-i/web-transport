# @moq/web-transport
A WebTransport polyfill backed by the native `web-transport-quinn` implementation.

## Usage
```ts
import WebTransport, { install } from "@moq/web-transport";

// Optional global polyfill
install();

const transport = new WebTransport("https://localhost:4433");
await transport.ready;

const stream = await transport.createBidirectionalStream();
const writer = stream.writable.getWriter();
await writer.write(new Uint8Array([1, 2, 3]));
await writer.close();
```

## Notes
- Requires a native build (Node-API / N-API). Use `npm run build:native` or `bun run build:native`.
- Works in Node.js and Bun (both support N-API modules).
