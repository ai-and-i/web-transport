import { native, type ConnectOptions, type NativeCloseInfo, type NativeSession } from "./native.ts";
import { Datagrams } from "./datagrams.ts";
import { createReadableStream, createWritableStream, wrapBidirectionalStream } from "./streams.ts";
import { toUint8Array } from "./util.ts";

export default class WebTransportQuinn implements WebTransport {
	readonly ready: Promise<void>;
	readonly closed: Promise<WebTransportCloseInfo>;
	readonly incomingBidirectionalStreams: ReadableStream<WebTransportBidirectionalStream>;
	readonly incomingUnidirectionalStreams: ReadableStream<ReadableStream<Uint8Array>>;
	readonly datagrams: WebTransportDatagramDuplexStream;
	#congestionControl: string;

	#sessionPromise: Promise<NativeSession>;
	#session?: NativeSession;
	#closedResolve!: (info: WebTransportCloseInfo) => void;
	#closeInfo?: WebTransportCloseInfo;
	#closed = false;

	constructor(url: string | URL, options?: WebTransportOptions) {
		const connectOptions = normalizeOptions(options);
		const urlString = typeof url === "string" ? url : url.toString();
		this.#congestionControl = options?.congestionControl ?? "default";

		this.#sessionPromise = native.connect(urlString, connectOptions).then((session) => {
			this.#session = session;
			return session;
		});

		this.ready = this.#sessionPromise.then(() => undefined);

		this.closed = new Promise((resolve) => {
			this.#closedResolve = resolve;
		});

		this.incomingBidirectionalStreams = new ReadableStream<WebTransportBidirectionalStream>({
			start: (controller) => {
				this.#startIncomingBidirectional(controller);
			},
		});

		this.incomingUnidirectionalStreams = new ReadableStream<ReadableStream<Uint8Array>>({
			start: (controller) => {
				this.#startIncomingUnidirectional(controller);
			},
		});

		this.datagrams = new Datagrams(this.#sessionPromise);

		void this.#watchClosed();
	}

	get congestionControl(): string {
		return this.#congestionControl;
	}

	async createBidirectionalStream(): Promise<WebTransportBidirectionalStream> {
		const session = await this.#requireSession();
		const stream = await session.openBi();
		return wrapBidirectionalStream(stream);
	}

	async createUnidirectionalStream(): Promise<WritableStream<Uint8Array>> {
		const session = await this.#requireSession();
		const send = await session.openUni();
		return createWritableStream(send);
	}

	close(info?: WebTransportCloseInfo): void {
		if (this.#closed) return;
		const closeCode = info?.closeCode ?? 0;
		const reason = info?.reason ?? "";
		this.#closeInfo = { closeCode, reason };
		if (this.#session) {
			this.#session.close(closeCode, reason);
			return;
		}

		void this.#sessionPromise
			.then((session) => {
				session.close(closeCode, reason);
			})
			.catch(() => undefined);
	}

	async #requireSession(): Promise<NativeSession> {
		return await this.#sessionPromise;
	}

	async #startIncomingBidirectional(
		controller: ReadableStreamDefaultController<WebTransportBidirectionalStream>,
	) {
		let session: NativeSession;
		try {
			session = await this.#requireSession();
		} catch (error) {
			controller.error(error);
			return;
		}

		for (;;) {
			let stream;
			try {
				stream = await session.acceptBi();
			} catch (error) {
				controller.error(error);
				return;
			}

			if (!stream) {
				controller.close();
				return;
			}

			controller.enqueue(wrapBidirectionalStream(stream));
		}
	}

	async #startIncomingUnidirectional(
		controller: ReadableStreamDefaultController<ReadableStream<Uint8Array>>,
	) {
		let session: NativeSession;
		try {
			session = await this.#requireSession();
		} catch (error) {
			controller.error(error);
			return;
		}

		for (;;) {
			let stream;
			try {
				stream = await session.acceptUni();
			} catch (error) {
				controller.error(error);
				return;
			}

			if (!stream) {
				controller.close();
				return;
			}

			controller.enqueue(createReadableStream(stream));
		}
	}

	async #watchClosed() {
		try {
			const session = await this.#requireSession();
			const info = await session.closed();
			this.#resolveClosed(info);
		} catch (error) {
			const reason = error instanceof Error ? error.message : String(error);
			this.#resolveClosed({ closeCode: 0, reason });
		}
	}

	#resolveClosed(info: WebTransportCloseInfo | NativeCloseInfo) {
		if (this.#closed) return;
		this.#closed = true;
		this.#closedResolve(
			this.#closeInfo ?? { closeCode: info.closeCode, reason: info.reason },
		);
	}
}

function normalizeOptions(options?: WebTransportOptions): ConnectOptions | undefined {
	if (!options) return undefined;
	const serverCertificateHashes = options.serverCertificateHashes?.map((hash) => {
		if (hash.algorithm !== "sha-256") {
			throw new Error(`Unsupported hash algorithm: ${hash.algorithm}`);
		}
		return toUint8Array(hash.value);
	});

	return {
		serverCertificateHashes,
		congestionControl: options.congestionControl,
	};
}
