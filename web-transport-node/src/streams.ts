import { toBuffer } from "./util.ts";
import type { NativeBiStream, NativeRecvStream, NativeSendStream } from "./native.ts";

const DEFAULT_READ_CHUNK = 64 * 1024;

export function wrapBidirectionalStream(stream: NativeBiStream): WebTransportBidirectionalStream {
	return {
		readable: createReadableStream(stream.recv),
		writable: createWritableStream(stream.send),
	};
}

export function createReadableStream(recv: NativeRecvStream): ReadableStream<Uint8Array> {
	return new ReadableStream<Uint8Array>({
		pull: async (controller) => {
			let chunk: Uint8Array | null;
			try {
				chunk = await recv.read(DEFAULT_READ_CHUNK);
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
		},
		cancel: () => {
			recv.stop(0);
		},
	});
}

export function createWritableStream(send: NativeSendStream): WritableStream<Uint8Array> {
	return new WritableStream<Uint8Array>({
		write: async (chunk) => {
			await send.write(toBuffer(chunk));
		},
		close: async () => {
			await send.finish();
		},
		abort: () => {
			send.reset(0);
		},
	});
}
