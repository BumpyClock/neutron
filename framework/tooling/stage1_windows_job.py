#!/usr/bin/env python3
# pyright: reportAttributeAccessIssue=false
"""Launch a Windows process in a kill-on-close Job Object before it runs."""

from __future__ import annotations

import ctypes
from ctypes import wintypes
import math
import os
import subprocess
import time
from os import PathLike
from typing import BinaryIO, Mapping


CREATE_NEW_PROCESS_GROUP = 0x00000200
EXTENDED_STARTUPINFO_PRESENT = 0x00080000
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000
JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION = 1
JOB_OBJECT_BASIC_PROCESS_ID_LIST = 3
JOB_OBJECT_EXTENDED_LIMIT_INFORMATION = 9
PROC_THREAD_ATTRIBUTE_HANDLE_LIST = 0x00020002
PROC_THREAD_ATTRIBUTE_JOB_LIST = 0x0002000D
STARTF_USESTDHANDLES = 0x00000100
CREATE_UNICODE_ENVIRONMENT = 0x00000400
ERROR_INSUFFICIENT_BUFFER = 122
ERROR_INVALID_PARAMETER = 87
ERROR_MORE_DATA = 234
PROCESS_QUERY_LIMITED_INFORMATION = 0x00001000
SYNCHRONIZE = 0x00100000
WAIT_OBJECT_0 = 0
WAIT_TIMEOUT = 258
WAIT_FAILED = 0xFFFFFFFF
INFINITE = 0xFFFFFFFF
_MAX_JOB_PROCESSES = 4096


class BasicAccountingInformation(ctypes.Structure):
    _fields_ = [
        ("total_user_time", ctypes.c_longlong),
        ("total_kernel_time", ctypes.c_longlong),
        ("this_period_total_user_time", ctypes.c_longlong),
        ("this_period_total_kernel_time", ctypes.c_longlong),
        ("total_page_fault_count", wintypes.DWORD),
        ("total_processes", wintypes.DWORD),
        ("active_processes", wintypes.DWORD),
        ("total_terminated_processes", wintypes.DWORD),
    ]


class BasicLimitInformation(ctypes.Structure):
    _fields_ = [
        ("per_process_user_time_limit", ctypes.c_longlong),
        ("per_job_user_time_limit", ctypes.c_longlong),
        ("limit_flags", wintypes.DWORD),
        ("minimum_working_set_size", ctypes.c_size_t),
        ("maximum_working_set_size", ctypes.c_size_t),
        ("active_process_limit", wintypes.DWORD),
        ("affinity", ctypes.c_size_t),
        ("priority_class", wintypes.DWORD),
        ("scheduling_class", wintypes.DWORD),
    ]


class IoCounters(ctypes.Structure):
    _fields_ = [
        ("read_operation_count", ctypes.c_ulonglong),
        ("write_operation_count", ctypes.c_ulonglong),
        ("other_operation_count", ctypes.c_ulonglong),
        ("read_transfer_count", ctypes.c_ulonglong),
        ("write_transfer_count", ctypes.c_ulonglong),
        ("other_transfer_count", ctypes.c_ulonglong),
    ]


class ExtendedLimitInformation(ctypes.Structure):
    _fields_ = [
        ("basic_limit_information", BasicLimitInformation),
        ("io_info", IoCounters),
        ("process_memory_limit", ctypes.c_size_t),
        ("job_memory_limit", ctypes.c_size_t),
        ("peak_process_memory_used", ctypes.c_size_t),
        ("peak_job_memory_used", ctypes.c_size_t),
    ]


class StartupInfo(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("lp_reserved", wintypes.LPWSTR),
        ("lp_desktop", wintypes.LPWSTR),
        ("lp_title", wintypes.LPWSTR),
        ("dw_x", wintypes.DWORD),
        ("dw_y", wintypes.DWORD),
        ("dw_x_size", wintypes.DWORD),
        ("dw_y_size", wintypes.DWORD),
        ("dw_x_count_chars", wintypes.DWORD),
        ("dw_y_count_chars", wintypes.DWORD),
        ("dw_fill_attribute", wintypes.DWORD),
        ("dw_flags", wintypes.DWORD),
        ("w_show_window", wintypes.WORD),
        ("cb_reserved2", wintypes.WORD),
        ("lp_reserved2", ctypes.POINTER(ctypes.c_byte)),
        ("h_std_input", wintypes.HANDLE),
        ("h_std_output", wintypes.HANDLE),
        ("h_std_error", wintypes.HANDLE),
    ]


class StartupInfoEx(ctypes.Structure):
    _fields_ = [
        ("startup_info", StartupInfo),
        ("attribute_list", ctypes.c_void_p),
    ]


class ProcessInformation(ctypes.Structure):
    _fields_ = [
        ("process_handle", wintypes.HANDLE),
        ("thread_handle", wintypes.HANDLE),
        ("process_id", wintypes.DWORD),
        ("thread_id", wintypes.DWORD),
    ]


def _kernel32() -> ctypes.WinDLL:
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.SetInformationJobObject.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        ctypes.c_void_p,
        wintypes.DWORD,
    ]
    kernel32.SetInformationJobObject.restype = wintypes.BOOL
    kernel32.QueryInformationJobObject.argtypes = [
        wintypes.HANDLE,
        wintypes.DWORD,
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.POINTER(wintypes.DWORD),
    ]
    kernel32.QueryInformationJobObject.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.InitializeProcThreadAttributeList.argtypes = [
        ctypes.c_void_p,
        wintypes.DWORD,
        wintypes.DWORD,
        ctypes.POINTER(ctypes.c_size_t),
    ]
    kernel32.InitializeProcThreadAttributeList.restype = wintypes.BOOL
    kernel32.UpdateProcThreadAttribute.argtypes = [
        ctypes.c_void_p,
        wintypes.DWORD,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.POINTER(ctypes.c_size_t),
    ]
    kernel32.UpdateProcThreadAttribute.restype = wintypes.BOOL
    kernel32.DeleteProcThreadAttributeList.argtypes = [ctypes.c_void_p]
    kernel32.DeleteProcThreadAttributeList.restype = None
    kernel32.CreateProcessW.argtypes = [
        wintypes.LPCWSTR,
        wintypes.LPWSTR,
        ctypes.c_void_p,
        ctypes.c_void_p,
        wintypes.BOOL,
        wintypes.DWORD,
        ctypes.c_void_p,
        wintypes.LPCWSTR,
        ctypes.POINTER(StartupInfo),
        ctypes.POINTER(ProcessInformation),
    ]
    kernel32.CreateProcessW.restype = wintypes.BOOL
    kernel32.IsProcessInJob.argtypes = [
        wintypes.HANDLE,
        wintypes.HANDLE,
        ctypes.POINTER(wintypes.BOOL),
    ]
    kernel32.IsProcessInJob.restype = wintypes.BOOL
    kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.TerminateProcess.restype = wintypes.BOOL
    kernel32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.TerminateJobObject.restype = wintypes.BOOL
    kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel32.WaitForSingleObject.restype = wintypes.DWORD
    kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel32.GetExitCodeProcess.restype = wintypes.BOOL
    return kernel32


def _close_handle(kernel32: ctypes.WinDLL, handle: wintypes.HANDLE) -> bool:
    return bool(handle) and bool(kernel32.CloseHandle(handle))


def _close_or_terminate_job(kernel32: ctypes.WinDLL, handle: wintypes.HANDLE) -> bool:
    if _close_handle(kernel32, handle):
        return True
    kernel32.TerminateJobObject(handle, 1)
    return _close_handle(kernel32, handle)


def _create_kill_on_close_job(kernel32: ctypes.WinDLL) -> wintypes.HANDLE:
    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        raise ctypes.WinError(ctypes.get_last_error())
    limits = ExtendedLimitInformation()
    limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not kernel32.SetInformationJobObject(
        job,
        JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
        ctypes.byref(limits),
        ctypes.sizeof(limits),
    ):
        error = ctypes.get_last_error()
        _close_or_terminate_job(kernel32, job)
        raise ctypes.WinError(error)
    return job


def _require_process_in_job(
    kernel32: ctypes.WinDLL,
    process_handle: wintypes.HANDLE,
    job_handle: wintypes.HANDLE,
) -> None:
    in_job = wintypes.BOOL()
    if not kernel32.IsProcessInJob(process_handle, job_handle, ctypes.byref(in_job)):
        raise ctypes.WinError(ctypes.get_last_error())
    if not in_job.value:
        raise OSError("CreateProcessW did not place the scenario in its Job Object")


def _output_pipe() -> tuple[BinaryIO, int]:
    read_fd, write_fd = os.pipe()
    try:
        os.set_inheritable(read_fd, False)
        os.set_inheritable(write_fd, True)
        return os.fdopen(read_fd, "rb", buffering=0), write_fd
    except BaseException:
        os.close(read_fd)
        os.close(write_fd)
        raise


def _child_stdin(
    stdin: BinaryIO | int,
) -> tuple[BinaryIO | None, int]:
    if stdin == subprocess.PIPE:
        read_fd, write_fd = os.pipe()
        try:
            os.set_inheritable(read_fd, True)
            os.set_inheritable(write_fd, False)
            return os.fdopen(write_fd, "wb", buffering=0), read_fd
        except BaseException:
            os.close(read_fd)
            os.close(write_fd)
            raise
    if stdin == subprocess.DEVNULL:
        stdin_fd = os.open(os.devnull, os.O_RDONLY)
    elif isinstance(stdin, int):
        stdin_fd = os.dup(stdin)
    else:
        stdin_fd = os.dup(stdin.fileno())
    os.set_inheritable(stdin_fd, True)
    return None, stdin_fd


def _environment_block(environment: Mapping[str, str] | None) -> ctypes.Array[ctypes.c_wchar] | None:
    if environment is None:
        return None
    entries = []
    for key, value in sorted(environment.items(), key=lambda item: item[0].casefold()):
        if not key or "=" in key or "\0" in key or "\0" in value:
            raise ValueError(f"invalid environment entry {key!r}")
        entries.append(f"{key}={value}")
    return ctypes.create_unicode_buffer("\0".join(entries) + "\0\0")


class WindowsJobProcess:
    def __init__(
        self,
        kernel32: ctypes.WinDLL,
        process_handle: wintypes.HANDLE,
        process_id: int,
        stdin: BinaryIO | None,
        stdout: BinaryIO,
        stderr: BinaryIO,
        job_handle: wintypes.HANDLE,
        command: list[str],
    ) -> None:
        self._kernel32 = kernel32
        self._handle: wintypes.HANDLE | None = process_handle
        self._stage1_job_handle: wintypes.HANDLE | None = job_handle
        self.pid = process_id
        self.stdin = stdin
        self.stdout = stdout
        self.stderr = stderr
        self.args = command
        self.returncode: int | None = None

    def _collect_returncode(self) -> int:
        if self.returncode is not None:
            return self.returncode
        exit_code = wintypes.DWORD()
        if not self._kernel32.GetExitCodeProcess(self._handle, ctypes.byref(exit_code)):
            raise ctypes.WinError(ctypes.get_last_error())
        self.returncode = exit_code.value
        return self.returncode

    def poll(self) -> int | None:
        if self.returncode is not None:
            return self.returncode
        result = self._kernel32.WaitForSingleObject(self._handle, 0)
        if result == WAIT_TIMEOUT:
            return None
        if result == WAIT_OBJECT_0:
            return self._collect_returncode()
        if result == WAIT_FAILED:
            raise ctypes.WinError(ctypes.get_last_error())
        raise OSError(f"WaitForSingleObject returned {result}")

    def wait(self, timeout: float | None = None) -> int:
        if self.returncode is not None:
            return self.returncode
        if timeout is None:
            milliseconds = INFINITE
        else:
            milliseconds = min(max(0, math.ceil(timeout * 1000)), INFINITE - 1)
        result = self._kernel32.WaitForSingleObject(self._handle, milliseconds)
        if result == WAIT_TIMEOUT:
            raise subprocess.TimeoutExpired(self.args, timeout if timeout is not None else 0.0)
        if result == WAIT_OBJECT_0:
            return self._collect_returncode()
        if result == WAIT_FAILED:
            raise ctypes.WinError(ctypes.get_last_error())
        raise OSError(f"WaitForSingleObject returned {result}")

    def terminate_tree(self, *, deadline: float) -> bool:
        # PID-based taskkill is intentionally not a fallback: the target can exit and its PID can
        # be reused between a handle check and taskkill. The retained Job/process handles are the
        # only safe process identities; failure to terminate them is reported to the caller.
        return terminate_job(self, deadline=deadline)

    def close(self) -> None:
        close_job(self)
        if self._handle is not None:
            _close_handle(self._kernel32, self._handle)
            self._handle = None
        for stream_name in ("stdin", "stdout", "stderr"):
            stream = getattr(self, stream_name)
            if stream is not None:
                try:
                    stream.close()
                except OSError:
                    pass
                setattr(self, stream_name, None)

    def __del__(self) -> None:
        self.close()


def close_job(process: WindowsJobProcess) -> bool:
    handle = process._stage1_job_handle
    if handle is None:
        return False
    if not _close_or_terminate_job(process._kernel32, handle):
        return False
    process._stage1_job_handle = None
    return True


def _active_job_processes(
    kernel32: ctypes.WinDLL,
    handle: wintypes.HANDLE,
) -> int:
    accounting = BasicAccountingInformation()
    if not kernel32.QueryInformationJobObject(
        handle,
        JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION,
        ctypes.byref(accounting),
        ctypes.sizeof(accounting),
        None,
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    return accounting.active_processes


def _job_process_ids(
    kernel32: ctypes.WinDLL,
    handle: wintypes.HANDLE,
) -> set[int]:
    capacity = 16
    header_size = ctypes.sizeof(wintypes.DWORD) * 2
    while capacity <= _MAX_JOB_PROCESSES:
        buffer = ctypes.create_string_buffer(
            header_size + capacity * ctypes.sizeof(ctypes.c_size_t)
        )
        return_length = wintypes.DWORD()
        if kernel32.QueryInformationJobObject(
            handle,
            JOB_OBJECT_BASIC_PROCESS_ID_LIST,
            buffer,
            ctypes.sizeof(buffer),
            ctypes.byref(return_length),
        ):
            header = ctypes.cast(buffer, ctypes.POINTER(wintypes.DWORD))
            process_count = int(header[1])
            if process_count > capacity:
                raise OSError("Job process list exceeded its reported buffer capacity")
            process_ids = (ctypes.c_size_t * process_count).from_buffer(
                buffer,
                header_size,
            )
            return {int(process_id) for process_id in process_ids}

        error = ctypes.get_last_error()
        if error != ERROR_MORE_DATA:
            raise ctypes.WinError(error)
        header = ctypes.cast(buffer, ctypes.POINTER(wintypes.DWORD))
        assigned_processes = int(header[0])
        capacity = max(capacity * 2, assigned_processes)
    raise OSError(f"Job exceeded the {_MAX_JOB_PROCESSES}-process supervision limit")


def _track_job_processes(
    process: WindowsJobProcess,
    process_handles: dict[int, wintypes.HANDLE],
) -> bool:
    job_handle = process._stage1_job_handle
    if job_handle is None:
        return False
    for process_id in _job_process_ids(process._kernel32, job_handle):
        if process_id in process_handles:
            continue
        process_handle = process._kernel32.OpenProcess(
            SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            False,
            process_id,
        )
        if not process_handle:
            if ctypes.get_last_error() == ERROR_INVALID_PARAMETER:
                continue
            return False
        try:
            in_job = wintypes.BOOL()
            if not process._kernel32.IsProcessInJob(
                process_handle,
                job_handle,
                ctypes.byref(in_job),
            ):
                return False
            if not in_job.value:
                continue
            process_handles[process_id] = process_handle
            process_handle = None
        finally:
            if process_handle:
                _close_handle(process._kernel32, process_handle)
    return True


def terminate_job(process: WindowsJobProcess, *, deadline: float) -> bool:
    handle = process._stage1_job_handle
    if handle is None:
        return False

    process_handles: dict[int, wintypes.HANDLE] = {}
    try:
        try:
            initial_tracking_succeeded = _track_job_processes(
                process,
                process_handles,
            )
        except OSError:
            initial_tracking_succeeded = False
        termination_succeeded = bool(process._kernel32.TerminateJobObject(handle, 1))
        if not initial_tracking_succeeded or not termination_succeeded:
            return False

        while True:
            if not _track_job_processes(process, process_handles):
                return False
            for process_id, process_handle in tuple(process_handles.items()):
                result = process._kernel32.WaitForSingleObject(process_handle, 0)
                if result == WAIT_OBJECT_0:
                    if not _close_handle(process._kernel32, process_handle):
                        return False
                    del process_handles[process_id]
                elif result == WAIT_FAILED:
                    return False
                elif result != WAIT_TIMEOUT:
                    return False

            if (
                not process_handles
                and _active_job_processes(process._kernel32, handle) == 0
                and not _job_process_ids(process._kernel32, handle)
            ):
                break
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return False
            time.sleep(min(0.01, remaining))
    except OSError:
        return False
    finally:
        for process_handle in process_handles.values():
            _close_handle(process._kernel32, process_handle)

    if not _close_handle(process._kernel32, handle):
        return False
    process._stage1_job_handle = None
    return True


def start_process(
    command: list[str],
    *,
    stdin: BinaryIO | int = subprocess.DEVNULL,
    environment: Mapping[str, str] | None = None,
    cwd: str | PathLike[str] | None = None,
) -> WindowsJobProcess:
    if os.name != "nt":
        raise OSError("Windows Job Object launcher requires Windows")

    import msvcrt

    kernel32 = _kernel32()
    job = None
    parent_stdin = None
    stdout = None
    stderr = None
    stdout_write_fd = None
    stderr_write_fd = None
    stdin_fd = None
    attribute_list = None
    attribute_list_initialized = False
    process_info = ProcessInformation()
    try:
        job = _create_kill_on_close_job(kernel32)
        parent_stdin, stdin_fd = _child_stdin(stdin)
        stdout, stdout_write_fd = _output_pipe()
        stderr, stderr_write_fd = _output_pipe()

        attribute_list_size = ctypes.c_size_t()
        if kernel32.InitializeProcThreadAttributeList(
            None, 2, 0, ctypes.byref(attribute_list_size)
        ) or ctypes.get_last_error() != ERROR_INSUFFICIENT_BUFFER:
            raise ctypes.WinError(ctypes.get_last_error())
        if not attribute_list_size.value:
            raise OSError("InitializeProcThreadAttributeList returned an empty allocation size")
        attribute_list_buffer = ctypes.create_string_buffer(attribute_list_size.value)
        attribute_list = ctypes.cast(attribute_list_buffer, ctypes.c_void_p)
        if not kernel32.InitializeProcThreadAttributeList(
            attribute_list, 2, 0, ctypes.byref(attribute_list_size)
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        attribute_list_initialized = True

        job_handles = (wintypes.HANDLE * 1)(job)
        standard_handles = (wintypes.HANDLE * 3)(
            wintypes.HANDLE(msvcrt.get_osfhandle(stdin_fd)),
            wintypes.HANDLE(msvcrt.get_osfhandle(stdout_write_fd)),
            wintypes.HANDLE(msvcrt.get_osfhandle(stderr_write_fd)),
        )
        if not kernel32.UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_JOB_LIST,
            ctypes.cast(job_handles, ctypes.c_void_p),
            ctypes.sizeof(job_handles),
            None,
            None,
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        if not kernel32.UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            ctypes.cast(standard_handles, ctypes.c_void_p),
            ctypes.sizeof(standard_handles),
            None,
            None,
        ):
            raise ctypes.WinError(ctypes.get_last_error())

        startup_info = StartupInfoEx()
        startup_info.startup_info.cb = ctypes.sizeof(startup_info)
        startup_info.startup_info.dw_flags = STARTF_USESTDHANDLES
        startup_info.startup_info.h_std_input = standard_handles[0]
        startup_info.startup_info.h_std_output = standard_handles[1]
        startup_info.startup_info.h_std_error = standard_handles[2]
        startup_info.attribute_list = attribute_list
        command_line = ctypes.create_unicode_buffer(subprocess.list2cmdline(command))
        environment_buffer = _environment_block(environment)
        environment_pointer = (
            ctypes.cast(environment_buffer, ctypes.c_void_p)
            if environment_buffer is not None
            else None
        )
        creation_flags = (
            CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT
        )
        if not kernel32.CreateProcessW(
            None,
            command_line,
            None,
            None,
            True,
            creation_flags,
            environment_pointer,
            os.fspath(cwd) if cwd is not None else None,
            ctypes.byref(startup_info.startup_info),
            ctypes.byref(process_info),
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        _require_process_in_job(kernel32, process_info.process_handle, job)
        if not _close_handle(kernel32, process_info.thread_handle):
            raise ctypes.WinError(ctypes.get_last_error())
        process_info.thread_handle = None
        process = WindowsJobProcess(
            kernel32,
            process_info.process_handle,
            process_info.process_id,
            parent_stdin,
            stdout,
            stderr,
            job,
            command,
        )
        process_info.process_handle = None
        parent_stdin = None
        stdout = None
        stderr = None
        job = None
        return process
    finally:
        if attribute_list_initialized:
            kernel32.DeleteProcThreadAttributeList(attribute_list)
        for fd in (stdin_fd, stdout_write_fd, stderr_write_fd):
            if fd is not None:
                os.close(fd)
        for stream in (parent_stdin, stdout, stderr):
            if stream is not None:
                stream.close()
        if process_info.process_handle:
            if job and _close_or_terminate_job(kernel32, job):
                job = None
            kernel32.TerminateProcess(process_info.process_handle, 1)
        if process_info.thread_handle:
            _close_handle(kernel32, process_info.thread_handle)
        if process_info.process_handle:
            _close_handle(kernel32, process_info.process_handle)
        if job:
            _close_or_terminate_job(kernel32, job)
