/* Local finite test driver. Protocol setup/RTP/AEAD/retransmit remain upstream. */
static void *probe_tone_feedback(void *arg) {
    struct ap2cl_s *p = arg;
    uint64_t next = ap2_io_monotonic_ms() + 2000;
    while (!atomic_load(&p->tone_feedback_stop) && !atomic_load(&p->rtsp_dead)) {
        probe_volume_tick(p);
        if (ap2_io_monotonic_ms() < next) { Sleep(50); continue; }
        uint8_t *body = NULL; int size = 0;
        int status = ap2_rtsp_send(p, "POST", "/feedback", NULL, 0, NULL, &body, &size);
        free(body);
        ap2_io_status_line("[PROBE] AUDIO_FEEDBACK http=%d host=%s", status,p->device.address);
        if (status != 200 && (status > 0 || atomic_load(&p->rtsp_dead))) {
            atomic_store(&p->media_healthy, false);
            break;
        }
        next = ap2_io_monotonic_ms() + 2000;
    }
    return NULL;
}
static void probe_tone_feedback_stop(struct ap2cl_s *p) {
    if (!p) return;
    if (p->tone_feedback_started) {
        atomic_store(&p->tone_feedback_stop, true);
        pthread_join(p->tone_feedback_thread, NULL);
        p->tone_feedback_started = false;
    }
}
static bool probe_feedback_start(struct ap2cl_s *p) {
    if (p->tone_feedback_started) return true;
    atomic_store(&p->tone_feedback_stop,false);
    if (pthread_create(&p->tone_feedback_thread,NULL,probe_tone_feedback,p)) return false;
    p->tone_feedback_started=true;return true;
}
static bool probe_stream_healthy(struct ap2cl_s *p) {
    struct ap2cl_s *q=p->group_peer;
    return !atomic_load(&p->rtsp_dead) && atomic_load(&p->media_healthy) &&
        (!q || (!atomic_load(&q->rtsp_dead) && atomic_load(&q->media_healthy)));
}
static void probe_wait_until(HANDLE timer, uint64_t deadline_ms) {
    uint64_t now = ap2_io_monotonic_ms();
    if (now >= deadline_ms) return;
    LARGE_INTEGER due; due.QuadPart = -(LONGLONG)(deadline_ms - now) * 10000;
    if (SetWaitableTimer(timer, &due, 0, NULL, NULL, FALSE)) WaitForSingleObject(timer, INFINITE);
    else Sleep((DWORD)(deadline_ms - now));
}
static void probe_packet_report(struct ap2cl_s *p) {
    ap2_io_status_line("[PROBE] PACKET_STATS sent=%" PRIu64 " send_dropped=%" PRIu64
        " sync_dropped=%" PRIu64 " rtx_requested=%llu rtx_resent=%llu rtx_expired=%llu host=%s",
        p->audio_packets_sent,p->audio_packets_dropped,p->sync_packets_dropped,
        atomic_load(&p->rtx_requested),atomic_load(&p->rtx_answered),atomic_load(&p->rtx_expired),p->device.address);
}
/* Reads partial pipe writes without blocking indefinitely. Never substitutes
   silence for a stalled producer; EOF alone means a clean end of input. */
static int probe_pipe_read(struct ap2cl_s *p, void *buffer, DWORD wanted, DWORD timeout_ms) {
    HANDLE pipe = GetStdHandle(STD_INPUT_HANDLE);
    DWORD used = 0;
    uint64_t deadline = ap2_io_monotonic_ms() + timeout_ms;
    while (used < wanted) {
        DWORD available = 0;
        if (!PeekNamedPipe(pipe, NULL, 0, NULL, &available, NULL)) {
            if (GetLastError() == ERROR_BROKEN_PIPE) return (int)used;
            return -1;
        }
        if (!probe_stream_healthy(p)) return -1;
        if (!available) {
            if (ap2_io_monotonic_ms() >= deadline) {
                ap2_io_status_line("[PROBE] PCM_STALL timeout_ms=%lu", (unsigned long)timeout_ms);
                return -1;
            }
            Sleep(2); continue;
        }
        DWORD got = 0;
        DWORD count = available < wanted-used ? available : wanted-used;
        if (!ReadFile(pipe, (uint8_t *)buffer+used, count, &got, NULL) || !got) return -1;
        used += got;
    }
    return (int)used;
}
static bool probe_send_live(struct ap2cl_s *p, const probe_options *options) {
    p->phase = "live-audio";
    HANDLE timer = CreateWaitableTimerExW(NULL, NULL, 0x2, TIMER_ALL_ACCESS);
    if (!timer) timer = CreateWaitableTimerW(NULL, FALSE, NULL);
    if (!timer) return false;
    if (!probe_feedback_start(p)) { CloseHandle(timer); return false; }
    unsigned wanted_frames = ((unsigned)options->buffer_ms*44100+351999)/352000*352;
    uint32_t prebuffer[352*65] = {0}; /* <=512 ms, packet aligned */
    DWORD wanted_bytes = wanted_frames*4;
    ap2_io_status_line("[PROBE] PCM_READY rate=44100 bits=16 channels=2 prebuffer_frames=%u lead_ms=%d", wanted_frames, p->lead_ms);
    int bytes = probe_pipe_read(p, prebuffer, wanted_bytes, 8000);
    bool good = bytes > 0 && bytes % 4 == 0;
    bool eof = bytes >= 0 && bytes < (int)wanted_bytes;
    uint64_t input_frames = good ? (uint64_t)bytes/4 : 0;
    const uint64_t prebuffer_frames = input_frames;
    uint64_t sent_frames = 0, started = ap2_io_monotonic_ms();
    uint64_t next_report=started+5000;
    LARGE_INTEGER start_qpc, qpc_frequency;
    QueryPerformanceCounter(&start_qpc); QueryPerformanceFrequency(&qpc_frequency);
    ap2_io_status_line("[PROBE] PCM_CLOCK start_qpc=%lld frequency=%lld prebuffer_frames=%u lead_ms=%d",
                      (long long)start_qpc.QuadPart, (long long)qpc_frequency.QuadPart, wanted_frames, p->lead_ms);
    uint64_t tail_end = eof ? ((input_frames+351)/352)*352 + 3*44100 : UINT64_MAX;
    ap2_io_status_line("[PROBE] AUDIO_BEGIN rate=44100 bits=16 channels=2 source=pcm-stdin");
    if (p->group_peer) {
        uint64_t anchor=ap2_ptp_master_now_ns(p->ptp);
        struct ap2cl_s *q=p->group_peer;
        p->rt_anchor_wall0=q->rt_anchor_wall0=anchor;
        p->rt_anchor_pos0=q->rt_anchor_pos0=p->rtp_timestamp;
        p->rt_anchor_valid=q->rt_anchor_valid=true;
        ap2_io_status_line("[PROBE] GROUP_ANCHOR wall_ns=%" PRIu64 " rtp=%u lead_ms=%d",anchor,p->rtp_timestamp,p->lead_ms);
    }
    while (good && sent_frames < tail_end) {
        uint32_t stereo[352] = {0};
        if (sent_frames < prebuffer_frames) {
            size_t count = (size_t)(prebuffer_frames-sent_frames < 352 ? prebuffer_frames-sent_frames : 352);
            memcpy(stereo, prebuffer+sent_frames, count*4);
        } else if (!eof) {
            bytes = probe_pipe_read(p, stereo, sizeof(stereo), 1000);
            if (bytes < 0 || bytes % 4) { good = false; break; }
            input_frames += (uint64_t)bytes/4;
            if (bytes < (int)sizeof(stereo)) {
                eof = true;
                tail_end = sent_frames + (bytes ? 352 : 0) + 3*44100;
                ap2_io_status_line("[PROBE] PCM_EOF frames=%" PRIu64, input_frames);
            }
        }
        if (!probe_stream_healthy(p)) { good = false; break; }
        uint64_t due = started + sent_frames*1000/44100;
        /* A stalled capture must not cause a burst of late RTP packets. */
        if (ap2_io_monotonic_ms() > due+250) {
            ap2_io_status_line("[PROBE] PCM_LATE deadline_ms=%" PRIu64, due);
            good = false; break;
        }
        probe_wait_until(timer, due);
        if (ap2_native_send_chunk(p, (uint8_t *)stereo, 352) != AP2_SEND_SENT) { good = false; break; }
        if (p->group_peer && ap2_native_send_chunk(p->group_peer,(uint8_t *)stereo,352)!=AP2_SEND_SENT) {
            ap2_io_status_line("[PROBE] GROUP_FAILED member=1 phase=audio-send");good=false;break;
        }
        sent_frames += 352;
        if (ap2_io_monotonic_ms()>=next_report) {
            probe_packet_report(p);if (p->group_peer) probe_packet_report(p->group_peer);
            next_report=ap2_io_monotonic_ms()+5000;
        }
    }
    probe_tone_feedback_stop(p);
    probe_tone_feedback_stop(p->group_peer);
    probe_packet_report(p);
    if (p->group_peer) probe_packet_report(p->group_peer);
    CloseHandle(timer);
    ap2_io_status_line("[PROBE] AUDIO_STATS sent=%" PRIu64 " dropped=%" PRIu64 " sync=%" PRIu64 " input_frames=%" PRIu64,
                       p->audio_packets_sent, p->audio_packets_dropped, p->sync_packets_sent, input_frames);
    ap2_io_status_line("[PROBE] %s", good ? "AUDIO_TRANSPORT_OK" : "AUDIO_FAILED");
    return good;
}
static bool probe_send_audio(struct ap2cl_s *p, const probe_options *options) {
    p->phase = "audio";
    uint64_t pcm_frames = 0;
    FILE *source = options->pcm_file ? probe_pcm_open(options->pcm_file, &pcm_frames) : NULL;
    if (options->pcm_file && !source) return false;
    HANDLE timer = CreateWaitableTimerExW(NULL, NULL, 0x2 /* high resolution */, TIMER_ALL_ACCESS);
    if (!timer) timer = CreateWaitableTimerW(NULL, FALSE, NULL);
    if (!timer) { if (source) fclose(source); return false; }
    atomic_store(&p->tone_feedback_stop, false);
    int err = pthread_create(&p->tone_feedback_thread, NULL, probe_tone_feedback, p);
    if (err) { CloseHandle(timer); if (source) fclose(source); return false; }
    p->tone_feedback_started = true;
    /* 1 s silence, 4 s gently faded 440 Hz tone at -30 dBFS, 3 s silence.
       Preserve receiver volume. With a 2 s lead the tone has finished before
       teardown, while queued silence prevents an armed receiver underrun. */
    const uint64_t total_frames = source ? pcm_frames + 4*44100 : 8*44100;
    uint64_t sent_frames = 0, started = ap2_io_monotonic_ms();
    bool good = true;
    if (source) ap2_io_status_line("[PROBE] AUDIO_BEGIN rate=44100 bits=16 channels=2 source=pcm-file frames=%" PRIu64, pcm_frames);
    else ap2_io_status_line("[PROBE] AUDIO_BEGIN rate=44100 bits=16 channels=2 hz=440 tone_seconds=4 peak_dbfs=-30");
    while (sent_frames < total_frames) {
        if (atomic_load(&p->rtsp_dead) || !atomic_load(&p->media_healthy)) { good = false; break; }
        probe_wait_until(timer, started + sent_frames * 1000 / 44100);
        /* uint32_t storage is aligned for the pinned encoder's stereo reads. */
        uint32_t stereo[352] = {0};
        if (source) {
            uint64_t begin = sent_frames > 44100 ? sent_frames : 44100;
            uint64_t end = sent_frames+352 < 44100+pcm_frames ? sent_frames+352 : 44100+pcm_frames;
            if (end > begin && fread(stereo+(begin-sent_frames), 4, (size_t)(end-begin), source) != end-begin) {
                ap2_io_status_line("[PROBE] PCM_READ_FAILED"); good = false; break;
            }
        } else for (int i = 0; i < 352; i++) {
            double time = (double)(sent_frames + (uint64_t)i) / 44100.0;
            double value = 0;
            if (time >= 1 && time < 5) {
                double local = time - 1;
                double fade = fmin(1.0, fmin(local / 0.05, (4 - local) / 0.05));
                value = 1036.0 * fade * sin(2.0 * 3.141592653589793 * 440.0 * local);
            }
            int16_t sample = (int16_t)lrint(value);
            stereo[i] = (uint16_t)sample | ((uint32_t)(uint16_t)sample << 16);
        }
        if (ap2_native_send_chunk(p, (uint8_t *)stereo, 352) != AP2_SEND_SENT) {
            good = false; break;
        }
        sent_frames += 352;
    }
    good = good && !atomic_load(&p->rtsp_dead) && atomic_load(&p->media_healthy);
    probe_tone_feedback_stop(p);
    CloseHandle(timer);
    if (source) fclose(source);
    probe_packet_report(p);
    ap2_io_status_line("[PROBE] AUDIO_STATS sent=%" PRIu64 " dropped=%" PRIu64 " sync=%" PRIu64 " retransmitted=%llu",
                       p->audio_packets_sent, p->audio_packets_dropped, p->sync_packets_sent,
                       atomic_load(&p->rtx_answered));
    ap2_io_status_line("[PROBE] %s", good ? "AUDIO_TRANSPORT_OK" : "AUDIO_FAILED");
    return good;
}
