#include <ghostty/vt.h>

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define GRIZ_CELL_BOLD (1u << 0)
#define GRIZ_CELL_ITALIC (1u << 1)
#define GRIZ_CELL_FAINT (1u << 2)
#define GRIZ_CELL_BLINK (1u << 3)
#define GRIZ_CELL_CONCEALED (1u << 4)
#define GRIZ_CELL_STRIKETHROUGH (1u << 5)
#define GRIZ_CELL_OVERLINE (1u << 6)

struct GrizVtCell {
    size_t text_offset;
    size_t text_len;
    uint8_t width;
    uint8_t wide_spacer_head;
    uint8_t fg[3];
    uint8_t bg[3];
    uint8_t flags;
    uint8_t underline;
    uint8_t underline_color[3];
};

struct GrizVtSnapshot {
    uint16_t cols;
    uint16_t rows;
    uint16_t cursor_x;
    uint16_t cursor_y;
    uint8_t cursor_visible;
    uint8_t cursor_shape;
    uint8_t cursor_blinking;
    uint8_t cursor_color_has_value;
    uint8_t cursor_color[3];
    uint8_t application_cursor;
    uint8_t bracketed_paste;
    uint8_t mouse_tracking;
    uint64_t scrollback_offset;
    uint8_t* row_soft_wrapped;
    size_t row_soft_wrapped_len;
    struct GrizVtCell* cells;
    size_t cells_len;
    uint8_t* text;
    size_t text_len;
    size_t text_cap;
    uint8_t* title;
    size_t title_len;
};

struct GrizVt {
    GhosttyTerminal terminal;
    GhosttyRenderState render_state;
    GhosttyRenderStateRowIterator row_iterator;
    GhosttyRenderStateRowCells row_cells;
    GhosttyMouseEncoder mouse_encoder;
    GhosttyMouseEvent mouse_event;
    GhosttyKeyEncoder key_encoder;
    GhosttyKeyEvent key_event;
    bool mouse_configured;
    uint16_t mouse_modes;
    uint16_t mouse_cols;
    uint16_t mouse_rows;
    uint16_t mouse_cell_width;
    uint16_t mouse_cell_height;
    uint8_t* responses;
    size_t responses_len;
    size_t responses_cap;
    GhosttyResult responses_error;
    bool fail_next_capture_allocation;
    bool render_held;
    void (*program_status)(void*, const GhosttyTerminalProgramStatus*);
    void (*prompt_start)(void*);
    void* status_userdata;
};

static void griz_program_status(
    GhosttyTerminal terminal,
    void* userdata,
    const GhosttyTerminalProgramStatus* report
) {
    (void)terminal;
    struct GrizVt* vt = userdata;
    vt->program_status(vt->status_userdata, report);
}

static void griz_semantic_prompt(
    GhosttyTerminal terminal,
    void* userdata,
    const GhosttyTerminalSemanticPrompt* event
) {
    (void)terminal;
    struct GrizVt* vt = userdata;
    if (event->kind == GHOSTTY_SEMANTIC_PROMPT_PROMPT_START) {
        vt->prompt_start(vt->status_userdata);
    }
}

static void griz_render_hold(
    GhosttyTerminal terminal,
    void* userdata,
    bool held
) {
    (void)terminal;
    struct GrizVt* vt = userdata;
    if (vt != NULL) {
        vt->render_held = held;
    }
}

static void griz_capture_discard(struct GrizVt* vt) {
    free(vt->responses);
    vt->responses = NULL;
    vt->responses_len = 0;
    vt->responses_cap = 0;
    vt->responses_error = GHOSTTY_SUCCESS;
}

static void griz_capture_fail(struct GrizVt* vt, GhosttyResult error) {
    griz_capture_discard(vt);
    vt->responses_error = error;
}

static void griz_write_pty(
    GhosttyTerminal terminal,
    void* userdata,
    const uint8_t* data,
    size_t len
) {
    (void)terminal;
    struct GrizVt* vt = userdata;
    if (vt == NULL || vt->responses_error != GHOSTTY_SUCCESS || len == 0) {
        return;
    }
    if (data == NULL || len > SIZE_MAX - vt->responses_len) {
        griz_capture_fail(vt, GHOSTTY_OUT_OF_MEMORY);
        return;
    }

    size_t needed = vt->responses_len + len;
    if (needed > vt->responses_cap) {
        size_t capacity = vt->responses_cap == 0 ? 256 : vt->responses_cap;
        while (capacity < needed) {
            if (capacity > SIZE_MAX / 2) {
                capacity = needed;
                break;
            }
            capacity *= 2;
        }
        uint8_t* responses = NULL;
        if (vt->fail_next_capture_allocation) {
            vt->fail_next_capture_allocation = false;
        } else {
            responses = realloc(vt->responses, capacity);
        }
        if (responses == NULL) {
            griz_capture_fail(vt, GHOSTTY_OUT_OF_MEMORY);
            return;
        }
        vt->responses = responses;
        vt->responses_cap = capacity;
    }

    memcpy(vt->responses + vt->responses_len, data, len);
    vt->responses_len = needed;
}

static void griz_vt_free_partial(struct GrizVt* vt) {
    if (vt == NULL) {
        return;
    }
    free(vt->responses);
    ghostty_key_event_free(vt->key_event);
    ghostty_key_encoder_free(vt->key_encoder);
    ghostty_mouse_event_free(vt->mouse_event);
    ghostty_mouse_encoder_free(vt->mouse_encoder);
    ghostty_render_state_row_cells_free(vt->row_cells);
    ghostty_render_state_row_iterator_free(vt->row_iterator);
    ghostty_render_state_free(vt->render_state);
    ghostty_terminal_free(vt->terminal);
    free(vt);
}

int griz_vt_new(
    uint16_t cols,
    uint16_t rows,
    size_t scrollback_max_bytes,
    size_t scrollback_max_lines,
    struct GrizVt** out,
    void (*program_status)(void*, const GhosttyTerminalProgramStatus*),
    void (*prompt_start)(void*),
    void* status_userdata
) {
    if (out == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    *out = NULL;

    struct GrizVt* vt = calloc(1, sizeof(*vt));
    if (vt == NULL) {
        return GHOSTTY_OUT_OF_MEMORY;
    }

    vt->program_status = program_status;
    vt->prompt_start = prompt_start;
    vt->status_userdata = status_userdata;
    GhosttyResult result = ghostty_terminal_new(NULL, &vt->terminal, cols, rows);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_terminal_set(
        vt->terminal,
        GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
        &scrollback_max_bytes
    );
    if (result == GHOSTTY_SUCCESS) {
        result = ghostty_terminal_set(
            vt->terminal,
            GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
            &scrollback_max_lines
        );
    }
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_render_state_new(NULL, &vt->render_state);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_render_state_row_iterator_new(NULL, &vt->row_iterator);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_render_state_row_cells_new(NULL, &vt->row_cells);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_mouse_encoder_new(NULL, &vt->mouse_encoder);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    bool track_last_cell = true;
    result = ghostty_mouse_encoder_set(vt->mouse_encoder,
        GHOSTTY_MOUSE_ENCODER_OPT_TRACK_LAST_CELL, &track_last_cell);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_mouse_event_new(NULL, &vt->mouse_event);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_key_encoder_new(NULL, &vt->key_encoder);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_key_event_new(NULL, &vt->key_event);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }

    result = ghostty_terminal_set(vt->terminal, GHOSTTY_TERMINAL_OPT_USERDATA, vt);
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }
    result = ghostty_terminal_set(
        vt->terminal,
        GHOSTTY_TERMINAL_OPT_WRITE_PTY,
        (const void*)griz_write_pty
    );
    if (result == GHOSTTY_SUCCESS) {
        result = ghostty_terminal_set(
            vt->terminal,
            GHOSTTY_TERMINAL_OPT_RENDER_HOLD,
            (const void*)griz_render_hold
        );
    }
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }

    // Registering the native status hook also enables OSC 7501 feature replies
    // through write_pty, in stream order before a following DA response.
    result = ghostty_terminal_set(vt->terminal, GHOSTTY_TERMINAL_OPT_PROGRAM_STATUS,
                                  (const void*)griz_program_status);
    if (result == GHOSTTY_SUCCESS) {
        result = ghostty_terminal_set(vt->terminal, GHOSTTY_TERMINAL_OPT_SEMANTIC_PROMPT,
                                      (const void*)griz_semantic_prompt);
    }
    if (result != GHOSTTY_SUCCESS) {
        griz_vt_free_partial(vt);
        return result;
    }

    *out = vt;
    return GHOSTTY_SUCCESS;
}

void griz_vt_free(struct GrizVt* vt) {
    griz_vt_free_partial(vt);
}

void griz_vt_write(struct GrizVt* vt, const uint8_t* data, size_t len) {
    if (vt == NULL) {
        return;
    }
    ghostty_terminal_vt_write(vt->terminal, data, len);
}

int griz_vt_resize(struct GrizVt* vt, uint16_t cols, uint16_t rows) {
    if (vt == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    return ghostty_terminal_resize(vt->terminal, cols, rows, 0, 0);
}

uint8_t griz_vt_render_held(const struct GrizVt* vt) {
    return vt != NULL && vt->render_held ? 1 : 0;
}

int griz_vt_end_render_hold(struct GrizVt* vt) {
    if (vt == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    if (!vt->render_held) {
        return GHOSTTY_SUCCESS;
    }

    GhosttyTerminalModeConfig config = {
        .mode = GHOSTTY_MODE_SYNCHRONIZED_OUTPUT,
        .value = false,
    };
    GhosttyResult result = ghostty_terminal_set(
        vt->terminal,
        GHOSTTY_TERMINAL_OPT_MODE,
        &config
    );
    // Ghostty documents that host-side mode changes do not invoke render_hold.
    if (result == GHOSTTY_SUCCESS) {
        vt->render_held = false;
    }
    return result;
}

// Viewports are client-owned tracked references. Keep the terminal at the live
// area between captures so PTY writes, queries, and other clients are unaffected.
int griz_vt_set_viewport(struct GrizVt* vt, GhosttyTrackedGridRef position) {
    GhosttyTerminalScrollViewport scroll = {
        .tag = GHOSTTY_TERMINAL_SCROLL_VIEWPORT_BOTTOM,
    };
    GhosttyTerminalScreen screen;
    int result = ghostty_terminal_get(vt->terminal,
        GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN, &screen);
    if (result != GHOSTTY_SUCCESS) return result;
    if (position != NULL && screen == GHOSTTY_TERMINAL_SCREEN_PRIMARY) {
        GhosttyPointCoordinate point;
        result = ghostty_tracked_grid_ref_point(position, GHOSTTY_POINT_TAG_SCREEN, &point);
        if (result == GHOSTTY_SUCCESS) {
            scroll.tag = GHOSTTY_TERMINAL_SCROLL_VIEWPORT_ROW;
            scroll.value.row = point.y;
        } else if (result != GHOSTTY_NO_VALUE) {
            return result;
        }
    }
    return ghostty_terminal_scroll_viewport(vt->terminal, scroll);
}

int griz_vt_scroll(struct GrizVt* vt, int32_t rows, GhosttyTrackedGridRef* out) {
    *out = NULL;
    GhosttyTerminalScrollViewport scroll = {
        .tag = GHOSTTY_TERMINAL_SCROLL_VIEWPORT_DELTA,
        .value.delta = rows,
    };
    int result = ghostty_terminal_scroll_viewport(vt->terminal, scroll);
    if (result != GHOSTTY_SUCCESS) return result;
    GhosttyTerminalScrollbar bar;
    result = ghostty_terminal_get(vt->terminal, GHOSTTY_TERMINAL_DATA_SCROLLBAR, &bar);
    if (result != GHOSTTY_SUCCESS || bar.offset + bar.len >= bar.total) return result;
    GhosttyPoint point = {
        .tag = GHOSTTY_POINT_TAG_VIEWPORT,
        .value.coordinate = {.x = 0, .y = 0},
    };
    return ghostty_terminal_grid_ref_track(vt->terminal, point, out);
}

void griz_vt_viewport_free(GhosttyTrackedGridRef position) {
    ghostty_tracked_grid_ref_free(position);
}

static int griz_mode(struct GrizVt* vt, GhosttyMode mode, uint8_t* out);

// set_from_terminal clears motion deduplication. Refresh it only when modes
// actually change, not on every input event or ordinary application redraw.
static int griz_mouse_sync_modes(struct GrizVt* vt) {
    const GhosttyMode modes[] = {
        GHOSTTY_MODE_MOUSE_EVENT_X10,
        GHOSTTY_MODE_MOUSE_EVENT_NORMAL,
        GHOSTTY_MODE_MOUSE_EVENT_BUTTON,
        GHOSTTY_MODE_MOUSE_EVENT_ANY,
        GHOSTTY_MODE_MOUSE_FORMAT_UTF8,
        GHOSTTY_MODE_MOUSE_FORMAT_SGR,
        GHOSTTY_MODE_MOUSE_FORMAT_URXVT,
        GHOSTTY_MODE_MOUSE_FORMAT_SGR_PIXELS,
    };
    uint16_t bits = 0;
    for (size_t i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
        uint8_t enabled = 0;
        int result = griz_mode(vt, modes[i], &enabled);
        if (result != GHOSTTY_SUCCESS) return result;
        if (enabled) bits |= (uint16_t)(1u << i);
    }
    if (vt->mouse_configured && vt->mouse_modes == bits) return GHOSTTY_SUCCESS;
    int result = ghostty_mouse_encoder_set_from_terminal(vt->mouse_encoder, vt->terminal);
    if (result == GHOSTTY_SUCCESS) {
        vt->mouse_modes = bits;
        vt->mouse_configured = true;
    }
    return result;
}

int griz_vt_mouse_input(
    struct GrizVt* vt,
    uint8_t action,
    int8_t button,
    float x,
    float y,
    uint16_t cell_width,
    uint16_t cell_height,
    uint8_t shift,
    uint8_t alt,
    uint8_t ctrl,
    uint8_t** out_data,
    size_t* out_len
) {
    if (vt == NULL || out_data == NULL || out_len == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    *out_data = NULL;
    *out_len = 0;

    GhosttyMouseAction ghostty_action;
    switch (action) {
    case 0: ghostty_action = GHOSTTY_MOUSE_ACTION_PRESS; break;
    case 1: ghostty_action = GHOSTTY_MOUSE_ACTION_RELEASE; break;
    case 2: ghostty_action = GHOSTTY_MOUSE_ACTION_MOTION; break;
    default: return GHOSTTY_INVALID_VALUE;
    }

    GhosttyResult result = griz_mouse_sync_modes(vt);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    uint16_t cols = 0;
    uint16_t rows = 0;
    result = ghostty_terminal_get(vt->terminal, GHOSTTY_TERMINAL_DATA_COLS, &cols);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    result = ghostty_terminal_get(vt->terminal, GHOSTTY_TERMINAL_DATA_ROWS, &rows);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    if (vt->mouse_cols != cols || vt->mouse_rows != rows ||
        vt->mouse_cell_width != cell_width || vt->mouse_cell_height != cell_height) {
        GhosttyMouseEncoderSize size = GHOSTTY_INIT_SIZED(GhosttyMouseEncoderSize);
        size.screen_width = (uint32_t)cols * cell_width;
        size.screen_height = (uint32_t)rows * cell_height;
        size.cell_width = cell_width;
        size.cell_height = cell_height;
        result = ghostty_mouse_encoder_set(
            vt->mouse_encoder, GHOSTTY_MOUSE_ENCODER_OPT_SIZE, &size);
        if (result != GHOSTTY_SUCCESS) return result;
        result = ghostty_mouse_encoder_reset(vt->mouse_encoder);
        if (result != GHOSTTY_SUCCESS) return result;
        vt->mouse_cols = cols;
        vt->mouse_rows = rows;
        vt->mouse_cell_width = cell_width;
        vt->mouse_cell_height = cell_height;
    }

    bool any_button_pressed = button >= 0 && ghostty_action != GHOSTTY_MOUSE_ACTION_RELEASE;
    result = ghostty_mouse_encoder_set(
        vt->mouse_encoder,
        GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
        &any_button_pressed
    );
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    result = ghostty_mouse_event_set_action(vt->mouse_event, ghostty_action);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    if (button < 0) {
        result = ghostty_mouse_event_clear_button(vt->mouse_event);
    } else {
        GhosttyMouseButton ghostty_button;
        switch (button) {
        case 0: ghostty_button = GHOSTTY_MOUSE_BUTTON_LEFT; break;
        case 1: ghostty_button = GHOSTTY_MOUSE_BUTTON_RIGHT; break;
        case 2: ghostty_button = GHOSTTY_MOUSE_BUTTON_MIDDLE; break;
        case 3: ghostty_button = GHOSTTY_MOUSE_BUTTON_FOUR; break;
        case 4: ghostty_button = GHOSTTY_MOUSE_BUTTON_FIVE; break;
        default: return GHOSTTY_INVALID_VALUE;
        }
        result = ghostty_mouse_event_set_button(vt->mouse_event, ghostty_button);
    }
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    GhosttyMods mods = 0;
    if (shift) mods |= GHOSTTY_MODS_SHIFT;
    if (alt) mods |= GHOSTTY_MODS_ALT;
    if (ctrl) mods |= GHOSTTY_MODS_CTRL;
    result = ghostty_mouse_event_set_mods(vt->mouse_event, mods);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    GhosttyMousePosition position = {.x = x, .y = y};
    result = ghostty_mouse_event_set_position(vt->mouse_event, position);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    size_t needed = 0;
    result = ghostty_mouse_encoder_encode(
        vt->mouse_encoder,
        vt->mouse_event,
        NULL,
        0,
        &needed
    );
    if (result == GHOSTTY_SUCCESS && needed == 0) {
        return GHOSTTY_SUCCESS;
    }
    if (result != GHOSTTY_OUT_OF_SPACE || needed == 0) {
        return result;
    }

    uint8_t* data = malloc(needed);
    if (data == NULL) {
        return GHOSTTY_OUT_OF_MEMORY;
    }
    size_t written = 0;
    result = ghostty_mouse_encoder_encode(
        vt->mouse_encoder,
        vt->mouse_event,
        data,
        needed,
        &written
    );
    if (result != GHOSTTY_SUCCESS || written != needed) {
        free(data);
        return result == GHOSTTY_SUCCESS ? GHOSTTY_INVALID_VALUE : result;
    }
    *out_data = data;
    *out_len = written;
    return GHOSTTY_SUCCESS;
}

// This bundled API exposes a setter but no getter for modifyOtherKeys state 2.
// Compare Ghostty's own encoding of one probe with that option disabled, rather
// than tracking escape sequences ourselves. The next real key resets both the
// encoder from the terminal and every event field; this probe is never sent.
static int griz_key_modify_other_active(struct GrizVt* vt, uint8_t* out) {
    *out = 0;
    GhosttyResult result = ghostty_key_encoder_set_from_terminal(
        vt->key_encoder,
        vt->terminal
    );
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_key_event_set_action(vt->key_event, GHOSTTY_KEY_ACTION_PRESS);
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_key_event_set_key(vt->key_event, GHOSTTY_KEY_A);
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_key_event_set_mods(vt->key_event, GHOSTTY_MODS_CTRL);
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_key_event_set_consumed_mods(vt->key_event, 0);
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_key_event_set_composing(vt->key_event, false);
    if (result != GHOSTTY_SUCCESS) return result;
    static const uint8_t text[] = "a";
    result = ghostty_key_event_set_utf8(
        vt->key_event,
        (GhosttyString){.ptr = text, .len = 1}
    );
    if (result != GHOSTTY_SUCCESS) return result;

    uint8_t live[64];
    size_t live_len = 0;
    result = ghostty_key_encoder_encode(
        vt->key_encoder, vt->key_event, live, sizeof(live), &live_len);
    if (result != GHOSTTY_SUCCESS) return result;

    bool disabled = false;
    result = ghostty_key_encoder_set(
        vt->key_encoder,
        GHOSTTY_KEY_ENCODER_OPT_MODIFY_OTHER_KEYS_STATE_2,
        &disabled
    );
    if (result != GHOSTTY_SUCCESS) return result;
    uint8_t legacy[64];
    size_t legacy_len = 0;
    result = ghostty_key_encoder_encode(
        vt->key_encoder, vt->key_event, legacy, sizeof(legacy), &legacy_len);
    if (result != GHOSTTY_SUCCESS) return result;

    *out = live_len != legacy_len || memcmp(live, legacy, live_len) != 0;
    return GHOSTTY_SUCCESS;
}

int griz_vt_key_input(
    struct GrizVt* vt,
    uint8_t action,
    uint16_t key,
    uint32_t unshifted_codepoint,
    uint8_t consumed_alt,
    const uint8_t* text,
    size_t text_len,
    uint8_t shift,
    uint8_t alt,
    uint8_t ctrl,
    uint8_t super_key,
    uint8_t** out_data,
    size_t* out_len,
    uint8_t* out_kitty_flags,
    uint8_t* out_modify_other_keys
) {
    if (vt == NULL || out_data == NULL || out_len == NULL || out_kitty_flags == NULL ||
        out_modify_other_keys == NULL || (text_len != 0 && text == NULL)) {
        return GHOSTTY_INVALID_VALUE;
    }
    *out_data = NULL;
    *out_len = 0;
    *out_kitty_flags = 0;
    *out_modify_other_keys = 0;

    GhosttyKeyAction ghostty_action;
    switch (action) {
    case 0: ghostty_action = GHOSTTY_KEY_ACTION_PRESS; break;
    case 1: ghostty_action = GHOSTTY_KEY_ACTION_REPEAT; break;
    default: return GHOSTTY_INVALID_VALUE;
    }

    // Keyboard modes can change without a rendered Screen publication. The encoder is owned
    // beside this terminal and is synchronized immediately before every encoding operation.
    GhosttyResult result = ghostty_key_encoder_set_from_terminal(
        vt->key_encoder,
        vt->terminal
    );
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_terminal_get(
        vt->terminal,
        GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS,
        out_kitty_flags
    );
    if (result != GHOSTTY_SUCCESS) return result;

    result = ghostty_key_event_set_action(vt->key_event, ghostty_action);
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_key_event_set_key(vt->key_event, (GhosttyKey)key);
    if (result != GHOSTTY_SUCCESS) return result;

    GhosttyMods mods = 0;
    if (shift) mods |= GHOSTTY_MODS_SHIFT;
    if (alt) mods |= GHOSTTY_MODS_ALT;
    if (ctrl) mods |= GHOSTTY_MODS_CTRL;
    if (super_key) mods |= GHOSTTY_MODS_SUPER;
    result = ghostty_key_event_set_mods(vt->key_event, mods);
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_key_event_set_consumed_mods(vt->key_event, 0);
    if (result != GHOSTTY_SUCCESS) return result;
    result = ghostty_key_event_set_composing(vt->key_event, false);
    if (result != GHOSTTY_SUCCESS) return result;
    // Griz's macOS Option policy treats generated Unicode as text, not Meta.
    // Keep that consumed modifier out of Kitty's shortcut identity as well.
    if (consumed_alt) {
        result = ghostty_key_event_set_consumed_mods(vt->key_event, GHOSTTY_MODS_ALT);
        if (result != GHOSTTY_SUCCESS) return result;
    }
    // Reset this on every event: the event handle is reused, including for named keys.
    result = ghostty_key_event_set_unshifted_codepoint(vt->key_event, unshifted_codepoint);
    if (result != GHOSTTY_SUCCESS) return result;
    GhosttyString utf8 = {.ptr = text, .len = text_len};
    result = ghostty_key_event_set_utf8(vt->key_event, utf8);
    if (result != GHOSTTY_SUCCESS) return result;

    size_t needed = 0;
    result = ghostty_key_encoder_encode(
        vt->key_encoder,
        vt->key_event,
        NULL,
        0,
        &needed
    );
    if (result == GHOSTTY_SUCCESS && needed == 0) return GHOSTTY_SUCCESS;
    if (result != GHOSTTY_OUT_OF_SPACE || needed == 0 || needed > 4096) {
        return result == GHOSTTY_OUT_OF_SPACE && needed > 4096
            ? GHOSTTY_INVALID_VALUE
            : result;
    }

    uint8_t* data = malloc(needed);
    if (data == NULL) return GHOSTTY_OUT_OF_MEMORY;
    size_t written = 0;
    result = ghostty_key_encoder_encode(
        vt->key_encoder,
        vt->key_event,
        data,
        needed,
        &written
    );
    if (result != GHOSTTY_SUCCESS || written != needed) {
        free(data);
        return result == GHOSTTY_SUCCESS ? GHOSTTY_INVALID_VALUE : result;
    }
    *out_data = data;
    *out_len = written;
    if (key == GHOSTTY_KEY_ENTER && shift && *out_kitty_flags == 0) {
        result = griz_key_modify_other_active(vt, out_modify_other_keys);
        if (result != GHOSTTY_SUCCESS) {
            free(data);
            *out_data = NULL;
            *out_len = 0;
            return result;
        }
    }
    return GHOSTTY_SUCCESS;
}

static int griz_mode(struct GrizVt* vt, GhosttyMode mode, uint8_t* out) {
    GhosttyTerminalModeConfig config = {
        .mode = mode,
        .value = false,
    };
    GhosttyResult result = ghostty_terminal_get(
        vt->terminal,
        GHOSTTY_TERMINAL_DATA_MODE,
        &config
    );
    if (result == GHOSTTY_SUCCESS) {
        *out = config.value ? 1 : 0;
    }
    return result;
}

static int griz_reserve_text(struct GrizVtSnapshot* snapshot, size_t additional) {
    if (additional > SIZE_MAX - snapshot->text_len) {
        return GHOSTTY_OUT_OF_MEMORY;
    }
    size_t needed = snapshot->text_len + additional;
    if (needed <= snapshot->text_cap) {
        return GHOSTTY_SUCCESS;
    }

    size_t capacity = snapshot->text_cap == 0 ? 256 : snapshot->text_cap;
    while (capacity < needed) {
        if (capacity > SIZE_MAX / 2) {
            capacity = needed;
            break;
        }
        capacity *= 2;
    }
    uint8_t* text = realloc(snapshot->text, capacity);
    if (text == NULL) {
        return GHOSTTY_OUT_OF_MEMORY;
    }
    snapshot->text = text;
    snapshot->text_cap = capacity;
    return GHOSTTY_SUCCESS;
}

static int griz_cell_text(
    struct GrizVtSnapshot* snapshot,
    GhosttyRenderStateRowCells cells,
    struct GrizVtCell* out
) {
    GhosttyBuffer buffer = {
        .ptr = NULL,
        .cap = 0,
        .len = 0,
    };
    GhosttyResult result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
        &buffer
    );
    if (result == GHOSTTY_SUCCESS && buffer.len == 0) {
        out->text_offset = snapshot->text_len;
        out->text_len = 0;
        return GHOSTTY_SUCCESS;
    }
    if (result != GHOSTTY_OUT_OF_SPACE || buffer.len == 0) {
        return result;
    }

    result = griz_reserve_text(snapshot, buffer.len);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    size_t offset = snapshot->text_len;
    size_t required = buffer.len;
    buffer.ptr = snapshot->text + offset;
    buffer.cap = required;
    buffer.len = 0;
    result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
        &buffer
    );
    if (result != GHOSTTY_SUCCESS || buffer.len != required) {
        return result == GHOSTTY_SUCCESS ? GHOSTTY_INVALID_VALUE : result;
    }

    snapshot->text_len += buffer.len;
    out->text_offset = offset;
    out->text_len = buffer.len;
    return GHOSTTY_SUCCESS;
}

static void griz_rgb(uint8_t out[3], GhosttyColorRgb color) {
    out[0] = color.r;
    out[1] = color.g;
    out[2] = color.b;
}

static GhosttyColorRgb griz_style_color(
    GhosttyStyleColor color,
    const GhosttyRenderStateColors* colors,
    GhosttyColorRgb fallback
) {
    switch (color.tag) {
    case GHOSTTY_STYLE_COLOR_PALETTE:
        return colors->palette[color.value.palette];
    case GHOSTTY_STYLE_COLOR_RGB:
        return color.value.rgb;
    default:
        return fallback;
    }
}

static int griz_fill_cell(
    struct GrizVtSnapshot* snapshot,
    GhosttyRenderStateRowCells cells,
    const GhosttyRenderStateColors* colors,
    struct GrizVtCell* out
) {
    GhosttyCell raw = 0;
    GhosttyStyle style;
    GhosttyColorRgb fg;
    GhosttyColorRgb bg;
    GhosttyCellWide wide = GHOSTTY_CELL_WIDE_NARROW;

    GhosttyResult result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_RAW,
        &raw
    );
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    result = ghostty_cell_get(raw, GHOSTTY_CELL_DATA_WIDE, &wide);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE,
        &style
    );
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR,
        &fg
    );
    if (result == GHOSTTY_NO_VALUE) {
        fg = colors->foreground;
    } else if (result != GHOSTTY_SUCCESS) {
        return result;
    }
    result = ghostty_render_state_row_cells_get(
        cells,
        GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR,
        &bg
    );
    if (result == GHOSTTY_NO_VALUE) {
        bg = colors->background;
    } else if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    if ((style.flags & GHOSTTY_STYLE_FLAG_INVERSE) != 0) {
        GhosttyColorRgb swap = fg;
        fg = bg;
        bg = swap;
    }

    griz_rgb(out->fg, fg);
    griz_rgb(out->bg, bg);
    out->width = wide == GHOSTTY_CELL_WIDE_WIDE
        ? 2
        : (wide == GHOSTTY_CELL_WIDE_SPACER_TAIL ? 0 : 1);
    out->wide_spacer_head = wide == GHOSTTY_CELL_WIDE_SPACER_HEAD ? 1 : 0;
    out->flags = 0;
    if ((style.flags & GHOSTTY_STYLE_FLAG_BOLD) != 0) {
        out->flags |= GRIZ_CELL_BOLD;
    }
    if ((style.flags & GHOSTTY_STYLE_FLAG_ITALIC) != 0) {
        out->flags |= GRIZ_CELL_ITALIC;
    }
    if ((style.flags & GHOSTTY_STYLE_FLAG_FAINT) != 0) {
        out->flags |= GRIZ_CELL_FAINT;
    }
    if ((style.flags & GHOSTTY_STYLE_FLAG_BLINK) != 0) {
        out->flags |= GRIZ_CELL_BLINK;
    }
    if ((style.flags & GHOSTTY_STYLE_FLAG_INVISIBLE) != 0) {
        out->flags |= GRIZ_CELL_CONCEALED;
    }
    if ((style.flags & GHOSTTY_STYLE_FLAG_STRIKETHROUGH) != 0) {
        out->flags |= GRIZ_CELL_STRIKETHROUGH;
    }
    if ((style.flags & GHOSTTY_STYLE_FLAG_OVERLINE) != 0) {
        out->flags |= GRIZ_CELL_OVERLINE;
    }
    out->underline = (uint8_t)GHOSTTY_STYLE_UNDERLINE(style);
    griz_rgb(
        out->underline_color,
        griz_style_color(style.underline_color, colors, fg)
    );

    return griz_cell_text(snapshot, cells, out);
}

void griz_vt_snapshot_free(struct GrizVtSnapshot* snapshot) {
    if (snapshot == NULL) {
        return;
    }
    free(snapshot->row_soft_wrapped);
    free(snapshot->cells);
    free(snapshot->text);
    free(snapshot->title);
    free(snapshot);
}

int griz_vt_snapshot(struct GrizVt* vt, struct GrizVtSnapshot** out) {
    if (vt == NULL || out == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    *out = NULL;

    GhosttyResult result = ghostty_render_state_update(vt->render_state, vt->terminal);
    if (result != GHOSTTY_SUCCESS) {
        return result;
    }

    struct GrizVtSnapshot* snapshot = calloc(1, sizeof(*snapshot));
    if (snapshot == NULL) {
        return GHOSTTY_OUT_OF_MEMORY;
    }

    GhosttyRenderStateColors colors = GHOSTTY_INIT_SIZED(GhosttyRenderStateColors);
    GhosttyRenderStateCursor cursor = GHOSTTY_INIT_SIZED(GhosttyRenderStateCursor);
    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_COLS,
        &snapshot->cols
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_ROWS,
        &snapshot->rows
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_COLORS,
        &colors
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_CURSOR,
        &cursor
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }

    GhosttyTerminalScrollbar bar;
    result = ghostty_terminal_get(vt->terminal, GHOSTTY_TERMINAL_DATA_SCROLLBAR, &bar);
    if (result != GHOSTTY_SUCCESS) goto fail;
    snapshot->scrollback_offset = bar.total - bar.len - bar.offset;
    snapshot->cursor_visible = cursor.visible && cursor.viewport_has_value &&
        snapshot->scrollback_offset == 0 ? 1 : 0;
    // Private wire values shared with GrizVtSnapshot, not Ghostty's enum values.
    switch (cursor.visual_style) {
        case GHOSTTY_CURSOR_STYLE_BAR: snapshot->cursor_shape = 1; break;
        case GHOSTTY_CURSOR_STYLE_UNDERLINE: snapshot->cursor_shape = 2; break;
        case GHOSTTY_CURSOR_STYLE_BLOCK_HOLLOW: snapshot->cursor_shape = 3; break;
        default: snapshot->cursor_shape = 0; break;
    }
    snapshot->cursor_blinking = cursor.blinking ? 1 : 0;
    snapshot->cursor_color_has_value = colors.cursor_has_value ? 1 : 0;
    if (colors.cursor_has_value) {
        griz_rgb(snapshot->cursor_color, colors.cursor);
    }
    if (cursor.viewport_has_value) {
        snapshot->cursor_x = cursor.viewport_x;
        snapshot->cursor_y = cursor.viewport_y;
    }

    result = griz_mode(vt, GHOSTTY_MODE_CURSOR_KEYS, &snapshot->application_cursor);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    result = griz_mode(vt, GHOSTTY_MODE_BRACKETED_PASTE, &snapshot->bracketed_paste);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    uint8_t mouse_mode = 0;
    result = griz_mode(vt, GHOSTTY_MODE_MOUSE_EVENT_X10, &mouse_mode);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    snapshot->mouse_tracking = mouse_mode;
    result = griz_mode(vt, GHOSTTY_MODE_MOUSE_EVENT_NORMAL, &mouse_mode);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    snapshot->mouse_tracking |= mouse_mode;
    result = griz_mode(vt, GHOSTTY_MODE_MOUSE_EVENT_BUTTON, &mouse_mode);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    snapshot->mouse_tracking |= mouse_mode;
    result = griz_mode(vt, GHOSTTY_MODE_MOUSE_EVENT_ANY, &mouse_mode);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    snapshot->mouse_tracking |= mouse_mode;

    GhosttyString title;
    result = ghostty_terminal_get(vt->terminal, GHOSTTY_TERMINAL_DATA_TITLE, &title);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    if (title.len > 0) {
        snapshot->title = malloc(title.len);
        if (snapshot->title == NULL) {
            result = GHOSTTY_OUT_OF_MEMORY;
            goto fail;
        }
        memcpy(snapshot->title, title.ptr, title.len);
        snapshot->title_len = title.len;
    }

    snapshot->row_soft_wrapped_len = snapshot->rows;
    snapshot->row_soft_wrapped = calloc(
        snapshot->row_soft_wrapped_len,
        sizeof(*snapshot->row_soft_wrapped)
    );
    if (snapshot->row_soft_wrapped == NULL && snapshot->row_soft_wrapped_len != 0) {
        result = GHOSTTY_OUT_OF_MEMORY;
        goto fail;
    }

    snapshot->cells_len = (size_t)snapshot->cols * (size_t)snapshot->rows;
    snapshot->cells = calloc(snapshot->cells_len, sizeof(*snapshot->cells));
    if (snapshot->cells == NULL && snapshot->cells_len != 0) {
        result = GHOSTTY_OUT_OF_MEMORY;
        goto fail;
    }
    for (size_t i = 0; i < snapshot->cells_len; i++) {
        snapshot->cells[i].width = 1;
        griz_rgb(snapshot->cells[i].fg, colors.foreground);
        griz_rgb(snapshot->cells[i].bg, colors.background);
        griz_rgb(snapshot->cells[i].underline_color, colors.foreground);
    }

    result = ghostty_render_state_get(
        vt->render_state,
        GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR,
        &vt->row_iterator
    );
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }

    while (ghostty_render_state_row_iterator_next(vt->row_iterator)) {
        int32_t y = -1;
        result = ghostty_render_state_row_get(
            vt->row_iterator,
            GHOSTTY_RENDER_STATE_ROW_DATA_VIEWPORT_Y,
            &y
        );
        if (result != GHOSTTY_SUCCESS) {
            goto fail;
        }
        if (y < 0 || y >= snapshot->rows) {
            continue;
        }

        GhosttyRow row;
        bool soft_wrapped = false;
        result = ghostty_render_state_row_get(
            vt->row_iterator,
            GHOSTTY_RENDER_STATE_ROW_DATA_RAW,
            &row
        );
        if (result != GHOSTTY_SUCCESS) {
            goto fail;
        }
        result = ghostty_row_get(row, GHOSTTY_ROW_DATA_WRAP, &soft_wrapped);
        if (result != GHOSTTY_SUCCESS) {
            goto fail;
        }
        snapshot->row_soft_wrapped[y] = soft_wrapped ? 1 : 0;

        result = ghostty_render_state_row_get(
            vt->row_iterator,
            GHOSTTY_RENDER_STATE_ROW_DATA_CELLS,
            &vt->row_cells
        );
        if (result != GHOSTTY_SUCCESS) {
            goto fail;
        }

        uint16_t x = 0;
        while (ghostty_render_state_row_cells_next(vt->row_cells)) {
            if (x >= snapshot->cols) {
                result = GHOSTTY_INVALID_VALUE;
                goto fail;
            }
            struct GrizVtCell* cell = &snapshot->cells[(size_t)y * snapshot->cols + x];
            result = griz_fill_cell(snapshot, vt->row_cells, &colors, cell);
            if (result != GHOSTTY_SUCCESS) {
                goto fail;
            }
            x++;
        }
        if (x != snapshot->cols) {
            result = GHOSTTY_INVALID_VALUE;
            goto fail;
        }
    }

    result = ghostty_render_state_clean(vt->render_state);
    if (result != GHOSTTY_SUCCESS) {
        goto fail;
    }
    *out = snapshot;
    return GHOSTTY_SUCCESS;

fail:
    griz_vt_snapshot_free(snapshot);
    return result;
}

static bool griz_paste_read(void* userdata, GhosttyString mime, GhosttyWriter writer) {
    (void)mime;
    GhosttyString* text = userdata;
    if (text->len == 0) {
        return true;
    }
    return writer.write(writer.userdata, (const uint8_t*)text->ptr, text->len);
}

int griz_vt_paste(
    struct GrizVt* vt,
    const uint8_t* data,
    size_t len,
    uint8_t allow_unsafe,
    uint8_t** out_data,
    size_t* out_len,
    uint8_t* out_pasted
) {
    if (vt == NULL || (data == NULL && len != 0) || out_data == NULL ||
        out_len == NULL || out_pasted == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    *out_data = NULL;
    *out_len = 0;
    *out_pasted = 0;
    if (vt->responses_len != 0 || vt->responses_error != GHOSTTY_SUCCESS) {
        return GHOSTTY_INVALID_VALUE;
    }

    static const char mime_bytes[] = "text/plain;charset=utf-8";
    GhosttyString mime = {
        .ptr = (const uint8_t*)mime_bytes,
        .len = sizeof(mime_bytes) - 1,
    };
    GhosttyString text = {.ptr = data, .len = len};
    GhosttyPaste paste = GHOSTTY_INIT_SIZED(GhosttyPaste);
    paste.location = GHOSTTY_CLIPBOARD_LOCATION_STANDARD;
    paste.source = GHOSTTY_PASTE_SOURCE_CLIPBOARD;
    paste.mimes = &mime;
    paste.mimes_len = 1;
    paste.reader = (GhosttyMimeReader){.read = griz_paste_read, .userdata = &text};
    paste.allow_unsafe = allow_unsafe != 0;

    bool pasted = false;
    GhosttyResult result = ghostty_terminal_paste(vt->terminal, &paste, &pasted);
    if (vt->responses_error != GHOSTTY_SUCCESS) {
        result = vt->responses_error;
    }
    if (result != GHOSTTY_SUCCESS) {
        griz_capture_discard(vt);
        return result;
    }
    if (!pasted && vt->responses_len != 0) {
        griz_capture_discard(vt);
        return GHOSTTY_INVALID_VALUE;
    }

    *out_data = vt->responses;
    *out_len = vt->responses_len;
    *out_pasted = pasted ? 1 : 0;
    vt->responses = NULL;
    vt->responses_len = 0;
    vt->responses_cap = 0;
    return GHOSTTY_SUCCESS;
}

int griz_vt_take_responses(struct GrizVt* vt, uint8_t** data, size_t* len) {
    if (vt == NULL || data == NULL || len == NULL) {
        return GHOSTTY_INVALID_VALUE;
    }
    *data = NULL;
    *len = 0;
    if (vt->responses_error != GHOSTTY_SUCCESS) {
        GhosttyResult result = vt->responses_error;
        griz_capture_discard(vt);
        return result;
    }

    *data = vt->responses;
    *len = vt->responses_len;
    vt->responses = NULL;
    vt->responses_len = 0;
    vt->responses_cap = 0;
    return GHOSTTY_SUCCESS;
}

void griz_vt_fail_next_capture_allocation(struct GrizVt* vt) {
    if (vt != NULL) {
        vt->fail_next_capture_allocation = true;
    }
}

void griz_vt_bytes_free(uint8_t* data) {
    free(data);
}
