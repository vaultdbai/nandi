"""Nandi runtime: a sandboxed MCP tool server.

The container only executes tools. It never calls an LLM and never holds API
keys — the host (Rust) runs the agent loop and dispatches tool calls here over
MCP on stdio: `docker exec -i <container> python /opt/nandi/app.py --stdio`.
The container sits on an internal network with no published ports, so stdio
through Docker is the only way in.

File tools are confined to WORKDIR.
"""

import os
import signal
import subprocess
import sys
from pathlib import Path

from mcp.server.mcpserver import MCPServer

WORKDIR = Path("/agent/workdir").resolve()
# skill.md `shell_timeout_s`, passed by the host; 30s when it's absent
SHELL_TIMEOUT_SECONDS = int(os.environ.get("NANDI_SHELL_TIMEOUT_SECONDS") or 30)
# Keeps a runaway command from flooding the LLM context
MAX_OUTPUT_CHARS = 100_000

mcp = MCPServer(
    name="nandi-runtime",
    instructions=f"Sandboxed tools. File paths are relative to {WORKDIR}.",
)


def resolve_in_workdir(path: str) -> Path:
    """Resolves `path` (relative to WORKDIR, or absolute) and rejects anything
    outside WORKDIR, including escapes via `..` or symlinks."""
    candidate = Path(path)
    if not candidate.is_absolute():
        candidate = WORKDIR / candidate
    resolved = candidate.resolve()
    if resolved != WORKDIR and WORKDIR not in resolved.parents:
        raise ValueError(f"Path is outside {WORKDIR}: {path}")
    return resolved


def truncate(text: str) -> str:
    if len(text) <= MAX_OUTPUT_CHARS:
        return text
    omitted = len(text) - MAX_OUTPUT_CHARS
    return f"{text[:MAX_OUTPUT_CHARS]}\n[... {omitted} characters truncated]"


@mcp.tool()
def shell(cmd: str) -> dict:
    """Run a shell command in the workdir. Times out after the agent's shell
    timeout (30 seconds unless its skill.md sets `shell_timeout_s`)."""
    # New session so a timeout can kill the whole process group, not just sh
    proc = subprocess.Popen(
        cmd,
        shell=True,
        cwd=WORKDIR,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        errors="replace",
        start_new_session=True,
    )
    try:
        stdout, stderr = proc.communicate(timeout=SHELL_TIMEOUT_SECONDS)
        exit_code = proc.returncode
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        stdout, stderr = proc.communicate()
        stderr += f"\n[timed out after {SHELL_TIMEOUT_SECONDS}s]"
        exit_code = -1

    return {
        "stdout": truncate(stdout),
        "stderr": truncate(stderr),
        "exit_code": exit_code,
    }


@mcp.tool()
def read_file(path: str) -> str:
    """Read a UTF-8 text file from the workdir."""
    return truncate(resolve_in_workdir(path).read_text(errors="replace"))


@mcp.tool()
def write_file(path: str, content: str) -> bool:
    """Write a UTF-8 text file in the workdir, creating parent folders."""
    target = resolve_in_workdir(path)
    if target == WORKDIR:
        raise ValueError("Path must name a file, not the workdir itself")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content)
    return True


@mcp.tool()
def list_dir(path: str = ".") -> list[str]:
    """List a folder in the workdir. Subfolders end with '/'."""
    folder = resolve_in_workdir(path)
    return sorted(
        f"{entry.name}/" if entry.is_dir() else entry.name
        for entry in folder.iterdir()
    )


def main() -> None:
    if "--stdio" not in sys.argv:
        sys.exit("Run with --stdio (Nandi starts it with docker exec)")
    mcp.run(transport="stdio")


if __name__ == "__main__":
    main()
