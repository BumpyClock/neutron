/*
 * External clipboard reader for the private Weston Stage 1 fixture.
 *
 * The test-desktop shell does not activate newly created client surfaces. This
 * client uses weston_test only to focus its own surface, then reads the current
 * selection through the ordinary wl_data_device protocol and writes it to
 * stdout for the external clipboard harness.
 */

#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <wayland-client.h>

#include "weston-test-client-protocol.h"

struct clipboard_reader {
	struct wl_display *display;
	struct wl_compositor *compositor;
	struct wl_data_device_manager *data_device_manager;
	struct wl_data_device *data_device;
	struct wl_seat *seat;
	struct wl_surface *surface;
	struct weston_test *weston_test;
	struct wl_data_offer *selection;
	char *mime_type;
	int finished;
	int failed;
};

static void
copy_selection(struct clipboard_reader *reader)
{
	char buffer[4096];
	int pipe_fds[2];
	ssize_t count;

	if (!reader->selection || !reader->mime_type || reader->finished ||
	    reader->failed)
		return;

	if (pipe(pipe_fds) < 0) {
		fprintf(stderr, "failed to create clipboard pipe: %s\n",
			strerror(errno));
		reader->failed = 1;
		return;
	}

	wl_data_offer_receive(reader->selection, reader->mime_type, pipe_fds[1]);
	close(pipe_fds[1]);
	if (wl_display_flush(reader->display) < 0) {
		fprintf(stderr, "failed to flush clipboard receive request: %s\n",
			strerror(errno));
		close(pipe_fds[0]);
		reader->failed = 1;
		return;
	}

	for (;;) {
		count = read(pipe_fds[0], buffer, sizeof buffer);
		if (count > 0) {
			ssize_t written = 0;

			while (written < count) {
				ssize_t result = write(STDOUT_FILENO,
						       buffer + written,
						       (size_t) (count - written));
				if (result > 0) {
					written += result;
					continue;
				}
				if (result < 0 && errno == EINTR)
					continue;
				fprintf(stderr, "failed to write clipboard payload: %s\n",
					strerror(errno));
				reader->failed = 1;
				break;
			}
			if (reader->failed)
				break;
			continue;
		}
		if (count == 0) {
			reader->finished = 1;
			break;
		}
		if (errno == EINTR)
			continue;
		fprintf(stderr, "failed to read clipboard payload: %s\n",
			strerror(errno));
		reader->failed = 1;
		break;
	}

	close(pipe_fds[0]);
}

static void
data_offer_offer(void *data, struct wl_data_offer *offer,
		 const char *mime_type)
{
	struct clipboard_reader *reader = data;

	(void) offer;
	if (strcmp(mime_type, "text/plain;charset=utf-8") != 0)
		return;

	free(reader->mime_type);
	reader->mime_type = strdup(mime_type);
	if (!reader->mime_type) {
		fprintf(stderr, "failed to store clipboard MIME type\n");
		reader->failed = 1;
	}
}

static void
data_offer_source_actions(void *data, struct wl_data_offer *offer,
			  uint32_t source_actions)
{
	(void) data;
	(void) offer;
	(void) source_actions;
}

static void
data_offer_action(void *data, struct wl_data_offer *offer, uint32_t action)
{
	(void) data;
	(void) offer;
	(void) action;
}

static const struct wl_data_offer_listener data_offer_listener = {
	.offer = data_offer_offer,
	.source_actions = data_offer_source_actions,
	.action = data_offer_action,
};

static void
data_device_data_offer(void *data, struct wl_data_device *data_device,
		       struct wl_data_offer *offer)
{
	struct clipboard_reader *reader = data;

	(void) data_device;
	wl_data_offer_add_listener(offer, &data_offer_listener, reader);
}

static void
data_device_enter(void *data, struct wl_data_device *data_device,
		  uint32_t serial, struct wl_surface *surface,
		  wl_fixed_t x, wl_fixed_t y, struct wl_data_offer *offer)
{
	(void) data;
	(void) data_device;
	(void) serial;
	(void) surface;
	(void) x;
	(void) y;
	(void) offer;
}

static void
data_device_leave(void *data, struct wl_data_device *data_device)
{
	(void) data;
	(void) data_device;
}

static void
data_device_motion(void *data, struct wl_data_device *data_device,
		   uint32_t time, wl_fixed_t x, wl_fixed_t y)
{
	(void) data;
	(void) data_device;
	(void) time;
	(void) x;
	(void) y;
}

static void
data_device_drop(void *data, struct wl_data_device *data_device)
{
	(void) data;
	(void) data_device;
}

static void
data_device_selection(void *data, struct wl_data_device *data_device,
		      struct wl_data_offer *offer)
{
	struct clipboard_reader *reader = data;

	(void) data_device;
	reader->selection = offer;
	if (offer && !reader->mime_type) {
		fprintf(stderr, "clipboard selection has no UTF-8 text MIME type\n");
		reader->failed = 1;
		return;
	}
	copy_selection(reader);
}

static const struct wl_data_device_listener data_device_listener = {
	.data_offer = data_device_data_offer,
	.enter = data_device_enter,
	.leave = data_device_leave,
	.motion = data_device_motion,
	.drop = data_device_drop,
	.selection = data_device_selection,
};

static uint32_t
minimum_version(uint32_t advertised, uint32_t supported)
{
	return advertised < supported ? advertised : supported;
}

static void
registry_global(void *data, struct wl_registry *registry, uint32_t name,
		const char *interface, uint32_t version)
{
	struct clipboard_reader *reader = data;

	if (strcmp(interface, wl_compositor_interface.name) == 0) {
		reader->compositor = wl_registry_bind(
			registry, name, &wl_compositor_interface,
			minimum_version(version, 4));
	} else if (strcmp(interface, wl_data_device_manager_interface.name) == 0) {
		reader->data_device_manager = wl_registry_bind(
			registry, name, &wl_data_device_manager_interface,
			minimum_version(version, 3));
	} else if (strcmp(interface, wl_seat_interface.name) == 0) {
		reader->seat = wl_registry_bind(registry, name, &wl_seat_interface,
						minimum_version(version, 7));
	} else if (strcmp(interface, weston_test_interface.name) == 0) {
		reader->weston_test = wl_registry_bind(
			registry, name, &weston_test_interface,
			minimum_version(version, 1));
	}
}

static void
registry_global_remove(void *data, struct wl_registry *registry, uint32_t name)
{
	(void) data;
	(void) registry;
	(void) name;
}

static const struct wl_registry_listener registry_listener = {
	.global = registry_global,
	.global_remove = registry_global_remove,
};

int
main(void)
{
	struct clipboard_reader reader = { 0 };
	struct wl_registry *registry;
	int result = EXIT_FAILURE;

	reader.display = wl_display_connect(NULL);
	if (!reader.display) {
		fprintf(stderr, "failed to connect to Wayland compositor\n");
		return EXIT_FAILURE;
	}

	registry = wl_display_get_registry(reader.display);
	wl_registry_add_listener(registry, &registry_listener, &reader);
	if (wl_display_roundtrip(reader.display) < 0) {
		fprintf(stderr, "failed to read Wayland globals\n");
		goto out;
	}

	if (!reader.compositor || !reader.data_device_manager || !reader.seat ||
	    !reader.weston_test) {
		fprintf(stderr, "Wayland fixture is missing a required global\n");
		goto out;
	}

	reader.data_device = wl_data_device_manager_get_data_device(
		reader.data_device_manager, reader.seat);
	wl_data_device_add_listener(reader.data_device, &data_device_listener,
				    &reader);

	reader.surface = wl_compositor_create_surface(reader.compositor);
	wl_surface_commit(reader.surface);
	weston_test_activate_surface(reader.weston_test, reader.surface);

	while (!reader.finished && !reader.failed) {
		if (wl_display_dispatch(reader.display) < 0) {
			fprintf(stderr, "Wayland clipboard dispatch failed: %s\n",
				strerror(errno));
			reader.failed = 1;
		}
	}

	if (reader.finished)
		result = EXIT_SUCCESS;

out:
	free(reader.mime_type);
	wl_display_disconnect(reader.display);
	return result;
}
