#!/usr/bin/env python3
"""Bounded cross-platform process supervision for Stage 1 evidence commands."""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
import os
from os import PathLike
import signal
import subprocess
import sys
import threading
import time
from typing import BinaryIO, Mapping, Protocol, cast, runtime_checkable


DEFAULT_CLEANUP_SECONDS = 5.0
_POSIX_TERM_GRACE_SECONDS = 0.5
_POLL_INTERVAL_SECONDS = 0.01
_SUPERVISED_SESSION_ENV = "GPUI_STAGE1_SUPERVISED_SESSION"


@runtime_checkable
class ManagedProcess(Protocol):
    @property
    def args(self) -> list[str]: ...

    @property
    def pid(self) -> int: ...

    @property
    def returncode(self) -> int | None: ...

    @property
    def stdin(self) -> BinaryIO | None: ...

    @property
    def stdout(self) -> BinaryIO | None: ...

    @property
    def stderr(self) -> BinaryIO | None: ...

    def poll(self) -> int | None: ...

    def wait(self, timeout: float | None = None) -> int: ...


class PumpStartFailurePolicy(Enum):
    CLEANUP = "cleanup"
    DEFER_TO_OWNER = "defer_to_owner"


class OutputPumpStartError(RuntimeError):
    def __init__(
        self,
        message: str,
        stdout: bytearray,
        stderr: bytearray,
        threads: tuple[threading.Thread, ...],
    ) -> None:
        super().__init__(message)
        self.stdout = stdout
        self.stderr = stderr
        self.threads = threads


@dataclass(frozen=True)
class CaptureResult:
    returncode: int | None
    stdout: bytes
    stderr: bytes
    elapsed_seconds: float
    timed_out: bool
    cleanup_timed_out: bool


class PosixProcess:
    """Popen facade whose `poll` observes exit without reaping the session leader."""

    def __init__(
        self,
        process: subprocess.Popen[bytes],
        command: list[str],
        *,
        owns_session: bool,
    ) -> None:
        self._process = process
        self.args = command
        self.owns_session = owns_session
        self.pid = process.pid
        self.returncode: int | None = None

    @property
    def stdin(self) -> BinaryIO | None:
        return cast(BinaryIO | None, self._process.stdin)

    @property
    def stdout(self) -> BinaryIO | None:
        return cast(BinaryIO | None, self._process.stdout)

    @property
    def stderr(self) -> BinaryIO | None:
        return cast(BinaryIO | None, self._process.stderr)

    def poll(self) -> int | None:
        if self.returncode is not None:
            return self.returncode
        try:
            status = os.waitid(os.P_PID, self.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        except ChildProcessError:
            return self._process.returncode
        if status is None:
            return None
        if status.si_code == os.CLD_EXITED:
            return status.si_status
        return -status.si_status

    def wait(self, timeout: float | None = None) -> int:
        if self.returncode is None:
            self.returncode = self._process.wait(timeout=timeout)
        return self.returncode


def start_process(
    command: list[str],
    *,
    stdin: BinaryIO | int = subprocess.DEVNULL,
    environment: Mapping[str, str] | None = None,
    cwd: str | PathLike[str] | None = None,
) -> ManagedProcess:
    """Start a contained process; Windows Job membership exists before target code runs."""
    if os.name == "nt":
        import stage1_windows_job

        return stage1_windows_job.start_process(
            command,
            stdin=stdin,
            environment=environment,
            cwd=cwd,
        )
    child_environment = dict(os.environ if environment is None else environment)
    joins_supervised_session = (
        sys.platform == "linux" and os.environ.get(_SUPERVISED_SESSION_ENV) == "1"
    )
    if sys.platform == "linux":
        child_environment[_SUPERVISED_SESSION_ENV] = "1"
    process = subprocess.Popen(
        command,
        stdin=stdin,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=child_environment,
        cwd=cwd,
        start_new_session=not joins_supervised_session,
        process_group=0 if joins_supervised_session else None,
        bufsize=0,
    )
    return PosixProcess(
        process,
        command,
        owns_session=not joins_supervised_session,
    )


def _seconds_left(deadline: float) -> float:
    return max(0.0, deadline - time.monotonic())


def _observe_until(process: ManagedProcess, deadline: float) -> bool:
    while True:
        if process.poll() is not None:
            return True
        remaining = _seconds_left(deadline)
        if remaining <= 0:
            return False
        time.sleep(min(_POLL_INTERVAL_SECONDS, remaining))


def observe_process_exit(process: ManagedProcess, *, deadline: float) -> bool:
    """Observe root exit without reaping its process/session identity."""
    return _observe_until(process, deadline)


def _reap_until(process: ManagedProcess, deadline: float) -> bool:
    if process.returncode is not None:
        return True
    timeout = _seconds_left(deadline)
    if timeout <= 0:
        return False
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        return False
    return True


def _linux_session_process_groups(session_id: int) -> tuple[set[int], bool]:
    groups: set[int] = set()
    scan_succeeded = True
    try:
        entries = os.scandir("/proc")
    except OSError:
        return groups, False
    with entries:
        for entry in entries:
            if not entry.name.isdecimal():
                continue
            try:
                with open(f"{entry.path}/stat", encoding="utf-8") as stat_file:
                    stat = stat_file.read()
                fields = stat[stat.rfind(")") + 2 :].split()
                state = fields[0]
                process_group = int(fields[2])
                process_session = int(fields[3])
            except FileNotFoundError:
                continue
            except (OSError, ValueError, IndexError):
                scan_succeeded = False
                continue
            if process_session == session_id and state != "Z":
                groups.add(process_group)
    return groups, scan_succeeded


def _signal_process_tree(process: ManagedProcess, signal_number: int) -> bool:
    groups = {process.pid}
    scan_succeeded = True

    try:
        os.killpg(process.pid, signal_number)
    except ProcessLookupError:
        pass
    except PermissionError:
        if sys.platform != "darwin":
            scan_succeeded = False

    if sys.platform == "linux" and getattr(process, "owns_session", False):
        session_groups, session_scan_succeeded = _linux_session_process_groups(process.pid)
        groups.update(session_groups)
        scan_succeeded = scan_succeeded and session_scan_succeeded

    for process_group in sorted(groups - {process.pid}):
        try:
            os.killpg(process_group, signal_number)
        except ProcessLookupError:
            pass
        except PermissionError:
            scan_succeeded = False
    return scan_succeeded


def terminate_process_tree(
    process: ManagedProcess,
    *,
    deadline: float,
    graceful: bool,
) -> bool:
    """Terminate one process tree before reaping its identity, then wait to the deadline."""
    if os.name == "nt":
        job_terminated = process.terminate_tree(deadline=deadline)  # type: ignore[attr-defined]
        return job_terminated and _reap_until(process, deadline)

    termination_confirmed = True
    if graceful:
        termination_confirmed = _signal_process_tree(process, signal.SIGTERM)
        term_deadline = min(deadline, time.monotonic() + _POSIX_TERM_GRACE_SECONDS)
        _observe_until(process, term_deadline)
    # `PosixProcess.poll` used waitid(WNOWAIT), so an owned session leader still reserves
    # every process-group identity in that session while all current groups receive SIGKILL.
    termination_confirmed = (
        _signal_process_tree(process, signal.SIGKILL) and termination_confirmed
    )
    return _reap_until(process, deadline) and termination_confirmed


def _pump(stream: BinaryIO, output: bytearray) -> None:
    try:
        while True:
            chunk = stream.read(65536)
            if not chunk:
                return
            output.extend(chunk)
    except (OSError, ValueError):
        return


def start_output_pumps(
    process: ManagedProcess,
    *,
    failure_policy: PumpStartFailurePolicy = PumpStartFailurePolicy.CLEANUP,
) -> tuple[bytearray, bytearray, tuple[threading.Thread, threading.Thread]]:
    started_threads: list[threading.Thread] = []
    stdout = bytearray()
    stderr = bytearray()
    try:
        if process.stdout is None or process.stderr is None:
            raise ValueError("managed process must expose stdout and stderr pipes")
        threads = (
            threading.Thread(
                target=_pump,
                args=(process.stdout, stdout),
                daemon=True,
                name="stage1-stdout-pump",
            ),
            threading.Thread(
                target=_pump,
                args=(process.stderr, stderr),
                daemon=True,
                name="stage1-stderr-pump",
            ),
        )
        for thread in threads:
            thread.start()
            started_threads.append(thread)
        return stdout, stderr, threads
    except BaseException as error:
        if failure_policy is PumpStartFailurePolicy.DEFER_TO_OWNER:
            raise OutputPumpStartError(
                str(error),
                stdout,
                stderr,
                tuple(started_threads),
            ) from error
        cleanup_deadline = time.monotonic() + DEFAULT_CLEANUP_SECONDS
        cleanup_succeeded = finish_streaming_process(
            process,
            tuple(started_threads),
            cleanup_deadline=cleanup_deadline,
        )
        if not cleanup_succeeded:
            raise RuntimeError(
                f"{error}; output-pump startup cleanup was not confirmed"
            ) from error
        raise


def _close_stream_without_waiting(stream: BinaryIO | None) -> None:
    if stream is None:
        return
    try:
        stream.close()
    except (OSError, ValueError):
        pass


def close_process_streams(process: ManagedProcess) -> None:
    for stream in (process.stdin, process.stdout, process.stderr):
        _close_stream_without_waiting(stream)


def join_output_pumps(
    process: ManagedProcess,
    threads: tuple[threading.Thread, ...],
    *,
    deadline: float,
) -> bool:
    for thread in threads:
        thread.join(timeout=_seconds_left(deadline))
    if not any(thread.is_alive() for thread in threads):
        return True

    close_process_streams(process)
    for thread in threads:
        thread.join(timeout=min(0.05, _seconds_left(deadline)))
    return not any(thread.is_alive() for thread in threads)


def close_process(process: ManagedProcess) -> None:
    close_process_streams(process)
    close = getattr(process, "close", None)
    if close is not None:
        close()


def finish_streaming_processes(
    processes: tuple[
        tuple[ManagedProcess, tuple[threading.Thread, ...]],
        ...,
    ],
    *,
    cleanup_seconds: float = DEFAULT_CLEANUP_SECONDS,
    cleanup_deadline: float | None = None,
    graceful: bool = False,
) -> bool:
    """Stop multiple owned trees before reaping any root, sharing one cleanup deadline."""
    if cleanup_deadline is None:
        cleanup_deadline = time.monotonic() + cleanup_seconds
    termination_confirmed = True
    managed = tuple(process for process, _ in processes)

    if os.name == "nt":
        for process in managed:
            try:
                if not process.terminate_tree(  # type: ignore[attr-defined]
                    deadline=cleanup_deadline
                ):
                    termination_confirmed = False
            except (OSError, ValueError):
                termination_confirmed = False
    else:
        if graceful:
            for process in managed:
                if not _signal_process_tree(process, signal.SIGTERM):
                    termination_confirmed = False
            term_deadline = min(
                cleanup_deadline,
                time.monotonic() + _POSIX_TERM_GRACE_SECONDS,
            )
            while time.monotonic() < term_deadline:
                if all(process.poll() is not None for process in managed):
                    break
                time.sleep(min(_POLL_INTERVAL_SECONDS, _seconds_left(term_deadline)))

        for process in managed:
            # Every owned POSIX session leader remains unreaped here, so all current groups in
            # every owned session receive SIGKILL before any root identity can be released.
            if not _signal_process_tree(process, signal.SIGKILL):
                termination_confirmed = False

    reaped = True
    for process in managed:
        if not _reap_until(process, cleanup_deadline):
            reaped = False

    output_drained = True
    for process, threads in processes:
        if not join_output_pumps(process, threads, deadline=cleanup_deadline):
            output_drained = False
        close_process(process)

    return termination_confirmed and reaped and output_drained


def finish_streaming_process(
    process: ManagedProcess,
    threads: tuple[threading.Thread, ...],
    *,
    cleanup_seconds: float = DEFAULT_CLEANUP_SECONDS,
    cleanup_deadline: float | None = None,
    graceful: bool = False,
) -> bool:
    return finish_streaming_processes(
        ((process, threads),),
        cleanup_seconds=cleanup_seconds,
        cleanup_deadline=cleanup_deadline,
        graceful=graceful,
    )


def run_capture(
    command: list[str],
    *,
    timeout_seconds: float,
    cleanup_seconds: float = DEFAULT_CLEANUP_SECONDS,
    cleanup_deadline: float | None = None,
    stdin: BinaryIO | int = subprocess.DEVNULL,
    environment: Mapping[str, str] | None = None,
    cwd: str | PathLike[str] | None = None,
) -> CaptureResult:
    if timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be greater than zero")
    if cleanup_seconds <= 0:
        raise ValueError("cleanup_seconds must be greater than zero")
    if stdin == subprocess.PIPE:
        raise ValueError(
            "run_capture does not own pipe input; use start_process and close its stdin explicitly"
        )

    started = time.monotonic()
    execution_deadline = started + timeout_seconds
    process = start_process(
        command,
        stdin=stdin,
        environment=environment,
        cwd=cwd,
    )
    stdout, stderr, threads = start_output_pumps(process)
    timed_out = time.monotonic() >= execution_deadline
    if not timed_out:
        timed_out = not _observe_until(process, execution_deadline)
    if cleanup_deadline is None:
        cleanup_deadline = time.monotonic() + cleanup_seconds

    cleanup_timed_out = False
    try:
        process_stopped = terminate_process_tree(
            process,
            deadline=cleanup_deadline,
            graceful=timed_out,
        )
        output_drained = join_output_pumps(process, threads, deadline=cleanup_deadline)
        cleanup_timed_out = not process_stopped or not output_drained
        return CaptureResult(
            returncode=process.returncode,
            stdout=bytes(stdout),
            stderr=bytes(stderr),
            elapsed_seconds=time.monotonic() - started,
            timed_out=timed_out,
            cleanup_timed_out=cleanup_timed_out,
        )
    finally:
        close_process(process)
