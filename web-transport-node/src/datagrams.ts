import { toBuffer } from "./util.ts";
import type { NativeSession } from "./native.ts";

export class Datagrams implements WebTransportDatagramDuplexStream {
	incomingHighWaterMark = 1024;
	incomingMaxAge: number | null = null;
	outgoingHighWaterMark = 1024;
	outgoingMaxAge: number | null = null;
	readonly readable: ReadableStream<Uint8Array>;
	readonly writable: WritableStream<Uint8Array>;

	#sessionPromise: Promise<NativeSession>;
	#session?: NativeSession;
	#maxDatagramSize = 0;

	constructor(sessionPromise: Promise<NativeSession>) {
		this.#sessionPromise = sessionPromise;

		this.readable = new ReadableStream<Uint8Array>({
			start: (controller) => {
				this.#startReadable(controller);
			},
		});

		this.writable = new WritableStream<Uint8Array>({
			write: async (chunk) => {
				const session = await this.#getSession();
				await session.sendDatagram(toBuffer(chunk));
			},
		});

		void this.#init();
	}

	get maxDatagramSize(): number {
		return this.#maxDatagramSize;
	}

	async #init() {
		const session = await this.#getSession();
		this.#maxDatagramSize = await session.maxDatagramSize();
	}

	async #getSession(): Promise<NativeSession> {
		if (this.#session) return this.#session;
		this.#session = await this.#sessionPromise;
		return this.#session;
	}

	async #startReadable(controller: ReadableStreamDefaultController<Uint8Array>) {
		let session: NativeSession;
		try {
			session = await this.#getSession();
		} catch (error) {
			controller.error(error);
			return;
		}

		for (;;) {
			let chunk: Uint8Array | null;
			try {
				chunk = await session.recvDatagram();
			} catch (error) {
				controller.error(error);
				return;
			}

			if (!chunk) {
				controller.close();
				return;
			}

			if (chunk.byteLength > 0) {
				controller.enqueue(chunk);
			}
		}
	}
}
