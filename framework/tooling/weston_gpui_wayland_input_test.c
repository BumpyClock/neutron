/*
 * Conformance-only Weston client-test fixture for GPUI's Wayland input path.
 * Built inside the pinned Weston source tree; never installed.
 */

#include "config.h"

#include <errno.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "weston-test-fixture-compositor.h"
#include "weston-test-runner.h"

extern char **environ;

static enum test_result_code
fixture_setup(struct weston_test_harness *harness)
{
	struct compositor_setup setup;

	compositor_setup_defaults(&setup);
	setup.backend = WESTON_BACKEND_HEADLESS;
	setup.renderer = WESTON_RENDERER_PIXMAN;
	setup.shell = SHELL_TEST_DESKTOP;
	setup.width = 320;
	setup.height = 240;
	setup.refresh = 60000;
	setup.transform = WL_OUTPUT_TRANSFORM_NORMAL;

	return weston_test_harness_execute_as_client(harness, &setup);
}
DECLARE_FIXTURE_SETUP(fixture_setup);

static enum test_result_code
run_gpui_wayland_clipboard(struct wet_testsuite_data *suite_data)
{
	const char *session = getenv("GPUI_STAGE1_WAYLAND_CLIPBOARD_SESSION");
	const char *artifact_dir = getenv("GPUI_STAGE1_ARTIFACT_DIR");
	char result_path[4096];
	char result[8];
	char *const argv[] = { "bash", (char *) session, NULL };
	FILE *result_file;
	pid_t child;
	int status;
	int trailing;
	int ret;

	(void) suite_data;

	if (!session || session[0] == '\0') {
		testlog("GPUI_STAGE1_WAYLAND_CLIPBOARD_SESSION is not set.\n");
		return RESULT_HARD_ERROR;
	}
	if (!artifact_dir || artifact_dir[0] == '\0') {
		testlog("GPUI_STAGE1_ARTIFACT_DIR is not set.\n");
		return RESULT_HARD_ERROR;
	}
	ret = snprintf(result_path, sizeof result_path,
		       "%s/clipboard.fixture-result", artifact_dir);
	if (ret < 0 || (size_t) ret >= sizeof result_path) {
		testlog("GPUI Stage 1 artifact path is too long.\n");
		return RESULT_HARD_ERROR;
	}
	if (unlink(result_path) < 0 && errno != ENOENT) {
		testlog("Failed to clear GPUI Wayland clipboard result: %s.\n",
			strerror(errno));
		return RESULT_HARD_ERROR;
	}

	if (setenv("WAYLAND_DISPLAY", THIS_TEST_NAME, 1) < 0) {
		testlog("Failed to set WAYLAND_DISPLAY: %s.\n", strerror(errno));
		return RESULT_HARD_ERROR;
	}

	ret = posix_spawnp(&child, "bash", NULL, NULL, argv, environ);
	if (ret != 0) {
		testlog("Failed to spawn GPUI Wayland clipboard conformance: %s.\n", strerror(ret));
		return RESULT_HARD_ERROR;
	}

	do {
		ret = waitpid(child, &status, 0);
	} while (ret < 0 && errno == EINTR);
	if (ret < 0 && errno != ECHILD) {
		testlog("Failed to wait for GPUI Wayland clipboard conformance: %s.\n",
			strerror(errno));
		return RESULT_HARD_ERROR;
	}
	if (ret >= 0 && (!WIFEXITED(status) || WEXITSTATUS(status) != 0)) {
		testlog("GPUI Wayland clipboard conformance failed with wait status %d.\n",
			status);
		return RESULT_FAIL;
	}

	result_file = fopen(result_path, "r");
	if (!result_file) {
		testlog("GPUI Wayland clipboard conformance produced no success result: %s.\n",
			strerror(errno));
		return RESULT_FAIL;
	}
	if (!fgets(result, sizeof result, result_file)) {
		testlog("GPUI Wayland clipboard conformance produced no readable success result.\n");
		fclose(result_file);
		return RESULT_FAIL;
	}
	trailing = fgetc(result_file);
	if (strcmp(result, "passed\n") != 0 || trailing != EOF ||
	    ferror(result_file)) {
		testlog("GPUI Wayland clipboard conformance produced an invalid success result.\n");
		fclose(result_file);
		return RESULT_FAIL;
	}
	if (fclose(result_file) != 0) {
		testlog("Failed to close GPUI Wayland clipboard result: %s.\n",
			strerror(errno));
		return RESULT_HARD_ERROR;
	}

	return RESULT_OK;
}

DECLARE_TEST_LIST(TESTFN(run_gpui_wayland_clipboard));
