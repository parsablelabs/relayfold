import path from 'node:path';
import { createCodingTools } from '@earendil-works/pi-coding-agent';
import type { AgentTool } from '@earendil-works/pi-agent-core';
import type { TaskSandbox } from '../../../core/ports/TaskSandbox.js';

export function createSandboxCodingTools(sandbox: TaskSandbox): AgentTool<any>[] {
    const read = {
        readFile: (file: string) => sandbox.readFile(file),
        access: async (file: string) => {
            const result = await sandbox.exec(['/bin/sh', '-c', 'test -r "$1"', 'access', file]);
            if (result.exitCode !== 0) throw new Error(`Not readable: ${file}`);
        },
        detectImageMimeType: async (file: string) => {
            const bytes = await sandbox.readFile(file);
            if (bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return 'image/png';
            if (bytes[0] === 255 && bytes[1] === 216 && bytes[2] === 255) return 'image/jpeg';
            if (bytes.subarray(0, 3).toString() === 'GIF') return 'image/gif';
            if (bytes.subarray(8, 12).toString() === 'WEBP') return 'image/webp';
            return null;
        },
    };
    const write = {
        writeFile: (file: string, content: string) => sandbox.writeFile(file, content),
        mkdir: (directory: string) => sandbox.mkdir(directory),
    };
    return createCodingTools(sandbox.workspacePath, {
        read: { operations: read },
        write: { operations: write },
        edit: { operations: { ...read, writeFile: write.writeFile } },
        bash: {
            operations: {
                exec: async (command, cwd, options) => sandbox.exec(['/bin/bash', '-lc', command], {
                    cwd: path.posix.resolve(cwd),
                    onData: options.onData,
                    ...(options.signal ? { signal: options.signal } : {}),
                    ...(options.timeout ? { timeoutMs: options.timeout * 1000 } : {}),
                    // Pi supplies the host environment here; deliberately ignore it.
                }),
            },
        },
    });
}
