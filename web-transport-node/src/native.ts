import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export type ConnectOptions = {
	serverCertificateHashes?: Uint8Array[];
	congestionControl?: "default" | "throughput" | "low-latency";
};

export type NativeSendStream = {
	write(chunk: Uint8Array): Promise<number>;
	finish(): Promise<void>;
	reset(code: number): void;
	closed(): Promise<number | null>;
};

export type NativeRecvStream = {
	read(max: number): Promise<Uint8Array | null>;
	stop(code: number): void;
	closed(): Promise<number | null>;
};

export type NativeBiStream = {
	send: NativeSendStream;
	recv: NativeRecvStream;
};

export type NativeCloseInfo = {
	closeCode: number;
	reason: string;
};

export type NativeSession = {
	openBi(): Promise<NativeBiStream>;
	openUni(): Promise<NativeSendStream>;
	acceptBi(): Promise<NativeBiStream | null>;
	acceptUni(): Promise<NativeRecvStream | null>;
	sendDatagram(payload: Uint8Array): Promise<void>;
	recvDatagram(): Promise<Uint8Array | null>;
	maxDatagramSize(): Promise<number>;
	close(code: number, reason: string): void;
	closed(): Promise<NativeCloseInfo>;
};

export type NativeBindings = {
	connect(url: string, options?: ConnectOptions): Promise<NativeSession>;
};

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));

const candidates = [
	join(here, "native/index.node"),
	join(here, "native/web_transport_node.node"),
	join(here, "native/web_transport.node"),
	join(here, "../native/index.node"),
	join(here, "../native/web_transport_node.node"),
	join(here, "../native/web_transport.node"),
	join(here, "web_transport_node.node"),
	join(here, "web_transport.node"),
	join(here, "../web_transport_node.node"),
	join(here, "../web_transport.node"),
];

let bindings: NativeBindings | null = null;
for (const candidate of candidates) {
	if (!existsSync(candidate)) continue;
	bindings = require(candidate) as NativeBindings;
	break;
}

if (!bindings) {
	const tried = candidates.map((p) => `- ${p}`).join("\n");
	throw new Error(
		"@moq/web-transport native module not found. Build it first (npm run build:native).\n" +
			`Tried:\n${tried}`,
	);
}

export const native = bindings;
