"""Select a minimal Windows build from a patched copy of pinned upstream sources.

Protocol/platform changes belong in patches/ and local C adapters, never Python substitutions.
The upstream submodules and generated files are not edited in place.
"""
import pathlib
import re
import sys
from upstream_guard import UpstreamGuard

ADAPTER = pathlib.Path(__file__).resolve().parent


def function(name, source):
    pattern = re.compile(r'^(?:static[ \t]+)?[A-Za-z_]\w*(?:[ \t]+[A-Za-z_]\w*)*[ \t*]+'
                         + re.escape(name) + r'\([^;{}]*\)\s*\{', re.M)
    matches = list(pattern.finditer(source))
    if len(matches) != 1:
        raise ValueError(f'{name}: expected one definition, found {len(matches)}')
    start = matches[0].start()
    pos = matches[0].end() - 1
    depth, state = 0, 'code'
    i = pos
    while i < len(source):
        c, pair = source[i], source[i:i + 2]
        if state == 'code':
            if pair == '/*':
                state = 'comment'
                i += 1
            elif pair == '//':
                state = 'line'
                i += 1
            elif c == '"':
                state = 'string'
            elif c == "'":
                state = 'char'
            elif c == '{':
                depth += 1
            elif c == '}':
                depth -= 1
                if depth == 0:
                    return source[start:i + 1].strip()
        elif state == 'comment':
            if pair == '*/':
                state = 'code'
                i += 1
        elif state == 'line':
            if c == '\n':
                state = 'code'
        elif c == '\\':
            i += 1
        elif (state == 'string' and c == '"') or (state == 'char' and c == "'"):
            state = 'code'
        i += 1
    raise ValueError('Unclosed function ' + name)


def local(name):
    return (ADAPTER / name).read_text(encoding='utf-8')


def select_sources(root, out):
    client = (root / 'src/ap2_client.c').read_text(encoding='utf-8')
    names = '''ap2_env_enabled ap2_status_is_auth ap2_auth_error_kind ap2_set_connect_error
ap2_capture_response ap2_log_failed_response ap2_report_failed_exchange
ap2_rtsp_write_request ap2_rtsp_farewell_teardown ap2_mark_rtsp_dead ap2_rtsp_timeout_ms
ap2_rtsp_send_ex_unlocked ap2_rtsp_send_ex_tracked ap2_rtsp_send_ex ap2_rtsp_send
ap2_gen_uuid ap2_dacp_bytes ap2_colon_hex ap2_txt_hex_field ap2_txt_features
ap2_features_has_ptp ap2_splice_denied ap2_txt_field ap2_model_prefix ap2_txt_version_major
ap2_receiver_os_ge_27 ap2_follow_receiver_clock ap2_audio_format_code
ap2_parse_format_capability ap2_make_timing_peer ap2_native_open_socket
ap2_native_get_info ap2_native_reset_connection ap2_native_transient
ap2_native_pair_verify probe_pair_once ap2_native_connect'''.split()
    selected = []
    for name in names:
        body = function(name, client)
        if name == 'probe_pair_once':
            body += '\n' + local('auth_auto.inc')
        if name == 'ap2_native_connect':
            body += '\n'
        selected.append(body)
    for name in ('ap2_rtx_store', 'ap2_rtx_resend', 'ap2_rtx_thread_main', 'ap2_rtx_start', 'ap2_rtx_stop',
                 'ap2_send_sync_packet', 'ap2_send_sync_packet_ptp', 'ap2_encrypt_audio', 'ap2_native_send_chunk'):
        selected.append(function(name, client))
    selected.append(local('windows_audio.inc') + function('probe_audio_setup', client) + '\n')

    context = local('probe_context.inc')
    constants = '''AP2_CHACHA_TAG_SIZE AP2_FMT_ALAC_44100_16_2 AP2_FMT_ALAC_44100_24_2
AP2_FMT_ALAC_48000_16_2 AP2_FMT_ALAC_48000_24_2 AP2_RTSP_SETUP_TIMEOUT_MS
AP2_RTSP_CONTROL_TIMEOUT_MS AP2_RTSP_FEEDBACK_TIMEOUT_MS AP2_RTSP_METADATA_TIMEOUT_MS
AP2_RTSP_ARTWORK_TIMEOUT_MS AP2_RTSP_FAREWELL_TIMEOUT_MS AP2_FEEDBACK_MAX_CONSECUTIVE_MISSES
AP2_RTSP_RX_BUF_SIZE AP2_DIAG_RESPONSE_MAX AP2_DIAG_BODY_MAX AP2_UDP_SEND_TIMEOUT_MS'''.split()
    for name in constants:
        definitions = re.findall(r'^#define\s+' + name + r'\s+[^\n]+', client, re.M)
        if len(definitions) != 1:
            raise ValueError(f'{name}: expected one constant, found {len(definitions)}')
        context += definitions[0] + '\n'
    footer = ''.join(local(name) for name in ('events_entry.inc', 'volume_entry.inc',
                                             'audio_entry.inc', 'probe_entry.inc'))
    header = ('/* Generated from pinned airplay-cli.\n'
              ' * Copyright (C) 2024-2026 Music Assistant Contributors\n'
              ' * SPDX-License-Identifier: Apache-2.0\n'
              ' * Local changes: session extraction, Windows sockets and diagnostic markers.\n */\n')
    (out / 'probe_client.c').write_text(header + context + '\n\n'.join(selected) + footer, encoding='utf-8')

    for name in ('ap2_hap.c', 'ap2_hap.h', 'ap2_io.c', 'ap2_io.h', 'ap2_ptp.c', 'ap2_ptp.h',
                 'ap2_ptp_shm.h', 'ap2_plist.c', 'ap2_plist.h', 'ap2_bplist.cpp', 'ap2_bplist.h'):
        text = (root / 'src' / name).read_text(encoding='utf-8')
        if name == 'ap2_ptp.c':
            marker = '/* ---- PTP daemon ---- */'
            if text.count(marker) != 1:
                raise ValueError('PTP daemon selection boundary changed')
            text = text[:text.index(marker)]
        (out / name).write_text(text, encoding='utf-8')
    for name in ('bplist.cpp', 'bplist.h'):
        (out / name).write_text((root / 'libraop/src' / name).read_text(encoding='utf-8'), encoding='utf-8')
    header = ('/* Copyright (C) 2024-2026 Music Assistant Contributors\n'
              ' * SPDX-License-Identifier: Apache-2.0\n'
              ' * Selected from alac_ext.cpp; allocation/argument checks added. */\n'
              '#include <stdint.h>\n#include <stdlib.h>\n#include <stdbool.h>\n')
    raw = function('pcm_to_alac_raw', (root / 'src/alac_ext.cpp').read_text(encoding='utf-8'))
    (out / 'raw_alac.c').write_text(header + raw, encoding='utf-8')


def main():
    root, out = map(pathlib.Path, sys.argv[1:3])
    git = sys.argv[3] if len(sys.argv) > 3 else 'git'
    guard = UpstreamGuard(root, ADAPTER / 'upstream-manifest.json', git)
    guard.verify_revisions()
    with guard.patched_sources() as patched:
        out.mkdir(parents=True, exist_ok=True)
        select_sources(patched, out)
    guard.finish(out)
    print('Applied reviewed patches and selected pinned control/realtime source.')


if __name__ == '__main__':
    main()
