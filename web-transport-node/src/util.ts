import { Buffer } from "node:buffer";

export function toUint8Array(value: BufferSource): Uint8Array {
	if (value instanceof ArrayBuffer) return new Uint8Array(value);
	if (ArrayBuffer.isView(value)) {
		return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
	}
	throw new TypeError("Expected ArrayBuffer or ArrayBufferView");
}

export function toBuffer(value: Uint8Array | ArrayBufferView | ArrayBuffer): Buffer {
	if (Buffer.isBuffer(value)) return value;
	if (value instanceof Uint8Array) {
		return Buffer.from(value.buffer, value.byteOffset, value.byteLength);
	}
	if (value instanceof ArrayBuffer) return Buffer.from(value);
	if (ArrayBuffer.isView(value)) {
		return Buffer.from(value.buffer, value.byteOffset, value.byteLength);
	}
	return Buffer.from(value as Uint8Array);
}
