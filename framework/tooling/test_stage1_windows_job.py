#!/usr/bin/env python3
# pyright: reportArgumentType=false
"""Unit seams for the Windows atomic Job Object launcher."""

from __future__ import annotations

import ctypes
from ctypes import wintypes
import io
from pathlib import Path
import subprocess
import sys
import time
import unittest

sys.path.insert(0, str(Path(__file__).parent))
import stage1_windows_job as launcher  # noqa: E402


class FakeKernel32:
    def __init__(
        self,
        close_result: bool = True,
        active_process_counts: list[int] | None = None,
        process_id_lists: list[set[int]] | None = None,
        wait_result: int = launcher.WAIT_OBJECT_0,
        process_list_query_succeeds: bool = True,
    ) -> None:
        self.close_result = close_result
        self.active_process_counts = active_process_counts or [0]
        self.process_id_lists = process_id_lists or [{77}, {77}, set()]
        self.wait_result = wait_result
        self.process_list_query_succeeds = process_list_query_succeeds
        self.closed_handles: list[int] = []
        self.terminated_jobs: list[int] = []

    def CloseHandle(self, handle: int) -> bool:
        self.closed_handles.append(handle)
        return self.close_result

    def OpenProcess(self, access: int, inherit: bool, process_id: int) -> int:
        return process_id + 1000

    def IsProcessInJob(
        self,
        process_handle: int,
        job_handle: int,
        result: object,
    ) -> bool:
        ctypes.cast(result, ctypes.POINTER(wintypes.BOOL)).contents.value = True
        return True

    def TerminateJobObject(self, handle: int, exit_code: int) -> bool:
        self.terminated_jobs.append(handle)
        return True

    def WaitForSingleObject(self, handle: int, milliseconds: int) -> int:
        return self.wait_result

    def QueryInformationJobObject(
        self,
        handle: int,
        information_class: int,
        information: object,
        information_size: int,
        return_length: object,
    ) -> bool:
        if information_class == launcher.JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION:
            accounting = ctypes.cast(
                information,
                ctypes.POINTER(launcher.BasicAccountingInformation),
            ).contents
            accounting.active_processes = self.active_process_counts.pop(0)
            return True

        if not self.process_list_query_succeeds:
            raise OSError("process-list query failed")
        process_ids = self.process_id_lists.pop(0)
        header = ctypes.cast(information, ctypes.POINTER(wintypes.DWORD))
        header[0] = len(process_ids)
        header[1] = len(process_ids)
        values = (ctypes.c_size_t * len(process_ids)).from_buffer(
            information,
            ctypes.sizeof(wintypes.DWORD) * 2,
        )
        for index, process_id in enumerate(sorted(process_ids)):
            values[index] = process_id
        return True


class FakeProcess:
    def __init__(self, kernel32: FakeKernel32, job_handle: int | None) -> None:
        self._kernel32 = kernel32
        self._stage1_job_handle = job_handle


class WindowsJobLauncherTests(unittest.TestCase):
    def test_job_list_attribute_value_is_prelaunch_input_attribute(self) -> None:
        self.assertEqual(launcher.PROC_THREAD_ATTRIBUTE_JOB_LIST, 0x0002000D)

    def test_process_membership_postcondition_accepts_contained_child(self) -> None:
        class MembershipKernel32:
            def IsProcessInJob(self, process: int, job: int, result: object) -> bool:
                ctypes.cast(result, ctypes.POINTER(wintypes.BOOL)).contents.value = True
                return True

        launcher._require_process_in_job(MembershipKernel32(), 1, 2)

    def test_process_membership_postcondition_rejects_uncontained_child(self) -> None:
        class MembershipKernel32:
            def IsProcessInJob(self, process: int, job: int, result: object) -> bool:
                return True

        with self.assertRaises(OSError):
            launcher._require_process_in_job(MembershipKernel32(), 1, 2)

    def test_close_job_releases_handle_and_clears_process_reference(self) -> None:
        kernel32 = FakeKernel32()
        process = FakeProcess(kernel32, 123)

        self.assertTrue(launcher.close_job(process))
        self.assertEqual(kernel32.closed_handles, [123])
        self.assertIsNone(process._stage1_job_handle)

    def test_close_job_keeps_handle_when_close_and_termination_fail(self) -> None:
        kernel32 = FakeKernel32(close_result=False)
        process = FakeProcess(kernel32, 123)

        self.assertFalse(launcher.close_job(process))
        self.assertEqual(kernel32.closed_handles, [123, 123])
        self.assertEqual(kernel32.terminated_jobs, [123])
        self.assertEqual(process._stage1_job_handle, 123)

    def test_terminate_job_waits_for_empty_job_before_releasing_handle(self) -> None:
        kernel32 = FakeKernel32()
        process = FakeProcess(kernel32, 123)

        self.assertTrue(
            launcher.terminate_job(process, deadline=time.monotonic() + 1)
        )
        self.assertEqual(kernel32.terminated_jobs, [123])
        self.assertEqual(kernel32.closed_handles, [1077, 123])
        self.assertIsNone(process._stage1_job_handle)

    def test_tracking_failure_still_terminates_and_retains_job_identity(self) -> None:
        kernel32 = FakeKernel32(process_list_query_succeeds=False)
        process = FakeProcess(kernel32, 123)

        self.assertFalse(
            launcher.terminate_job(process, deadline=time.monotonic() + 1)
        )
        self.assertEqual(kernel32.terminated_jobs, [123])
        self.assertEqual(kernel32.closed_handles, [])
        self.assertEqual(process._stage1_job_handle, 123)

    def test_terminate_job_timeout_retains_job_identity(self) -> None:
        kernel32 = FakeKernel32(
            process_id_lists=[{77}, {77}],
            wait_result=launcher.WAIT_TIMEOUT,
        )
        process = FakeProcess(kernel32, 123)

        self.assertFalse(launcher.terminate_job(process, deadline=0))
        self.assertEqual(kernel32.terminated_jobs, [123])
        self.assertEqual(kernel32.closed_handles, [1077])
        self.assertEqual(process._stage1_job_handle, 123)

    def test_wait_reports_timeout_without_windows_runtime(self) -> None:
        class TimeoutKernel32:
            def WaitForSingleObject(self, handle: int, milliseconds: int) -> int:
                self.handle = handle
                self.milliseconds = milliseconds
                return launcher.WAIT_TIMEOUT

        kernel32 = TimeoutKernel32()
        process = launcher.WindowsJobProcess(
            kernel32,
            1,
            7,
            None,
            io.BytesIO(),
            io.BytesIO(),
            2,
            ["scenario"],
        )
        with self.assertRaises(subprocess.TimeoutExpired):
            process.wait(0.1)
        process._handle = None
        process._stage1_job_handle = None


if __name__ == "__main__":
    unittest.main()
