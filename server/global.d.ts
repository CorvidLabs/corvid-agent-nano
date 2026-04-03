/**
 * Minimal ambient declarations for Node.js-compatible modules used in the
 * plugin bridge.  These are provided as a shim so that `bun x tsc` can
 * type-check the server source without needing bun-types installed as a
 * devDependency.
 */

declare module "node:net" {
  interface NetConnectOpts {
    path?: string;
    host?: string;
    port?: number;
  }

  interface Socket {
    setEncoding(encoding: string): this;
    write(data: string | Uint8Array): boolean;
    destroy(): this;
    end(): this;
    readonly destroyed: boolean;
    readonly writableEnded: boolean;
    on(event: "connect", listener: () => void): this;
    on(event: "data", listener: (data: string) => void): this;
    on(event: "error", listener: (err: Error) => void): this;
    on(event: "close", listener: () => void): this;
    on(event: "drain", listener: () => void): this;
    once(event: string, listener: (...args: unknown[]) => void): this;
  }

  function connect(options: NetConnectOpts): Socket;
}

declare var Buffer: {
  from(data: unknown): { toString(encoding: string): string };
};
