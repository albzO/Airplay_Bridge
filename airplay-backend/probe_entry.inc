
static struct ap2cl_s *probe_context_create(const probe_options *options) {
    struct ap2cl_s *p = calloc(1, sizeof(*p));
    if (!p) return NULL;
    p->sock_fd = p->events_sock = -1;
    p->data_sock = p->ctrl_sock = -1;
    p->volume_sock=-1;
    p->device.address = (char *)options->host;
    p->device.port = options->port;
    p->device.name = (char *)options->name;
    p->device.txt_records = (char *)options->txt;
    p->password = (char *)options->password;
    p->request_password = options->request_password;
    p->password_arg = options->password_arg;
    p->auth_credentials = (char *)options->credentials;
    p->dacp_id = (char *)options->identity;
    p->active_remote = (char *)options->active_remote;
    p->iface = (char *)options->bind_ip;
    p->format = (ap2_audio_format_t){44100, 16, 2};
    p->ptp_forced = true;
    p->ptp_enabled = options->use_ptp;
    p->cseq = 1;
    p->phase = "tcp";
    atomic_init(&p->rtsp_dead, false);
    atomic_init(&p->feedback_failures, 0);
    atomic_init(&p->media_healthy, true);
    atomic_init(&p->rtp_offset, 0);
    atomic_init(&p->rtx_stop, false);
    atomic_init(&p->rtx_requested, 0);
    atomic_init(&p->rtx_answered, 0);
    atomic_init(&p->rtx_expired, 0);
    atomic_init(&p->tone_feedback_stop, false);
    p->lead_ms = options->lead_ms;
    p->rtp_timestamp = 441000;
    if (pthread_mutex_init(&p->rtsp_lock, NULL) != 0) { free(p); return NULL; }
    if (pthread_mutex_init(&p->rtx_lock, NULL) != 0) { pthread_mutex_destroy(&p->rtsp_lock); free(p); return NULL; }
    return p;
}
static void probe_context_destroy(struct ap2cl_s *p) {
    if (!p) return;
    probe_tone_feedback_stop(p);
    probe_events_stop(p);
    if (p->volume_sock>=0) close(p->volume_sock);
    ap2_rtx_stop(p);
    if (p->data_sock>=0) close(p->data_sock);
    if (p->ctrl_sock>=0) close(p->ctrl_sock);
    if (p->alac) alac_delete_encoder(p->alac);
    SecureZeroMemory(p->audio_key,sizeof(p->audio_key));
    pthread_mutex_destroy(&p->rtx_lock);
    if (p->ptp && !p->ptp_borrowed) ap2_ptp_destroy(p->ptp);
    if (p->events_sock>=0) close(p->events_sock);
    if (p->sock_fd>=0) close(p->sock_fd);
    if (p->hap) ap2_hap_destroy(p->hap);
    pthread_mutex_destroy(&p->rtsp_lock);
    free(p);
}
static int probe_run_stereo(const probe_options *options);
static int probe_failure_exit(struct ap2cl_s *p) {
    int code=p->probe_error;
    if (!code && p->connect_http_status==470) code=14;
    if (!code && (p->connect_http_status==401 || p->connect_http_status==403)) code=15;
    if (!code && p->connect_error==AP2_CONNECT_ERROR_AUTH_FAILED) code=13;
    if (!code) code=1;
    const char *name=code==1?"BACKEND_FAILED":code==16?"PAIRING_MAX_TRIES":code==17?"PAIRING_MAX_PEERS":code==18?"PAIRING_UNAVAILABLE":code==19?"PAIRING_BUSY":code==10?"PASSWORD_REJECTED":code==11?"PASSWORD_REQUIRED":code==12?"PAIRING_BACKOFF":code==14?"PAIRING_REQUIRED":code==15?"ACCESS_DENIED":code==13?"AUTH_REJECTED":"BACKEND_FAILED";
    if (!strcmp(name,"BACKEND_FAILED")) code=1;
    ap2_io_status_line("[PROBE] ERROR code=%s exit=%d host=%s phase=%s",name,code,p->device.address,p->phase);
    return code;
}
int probe_run(const probe_options *options) {
    if (options->peer_host) return probe_run_stereo(options);
    struct ap2cl_s *p=probe_context_create(options);
    if (!p) { ap2_io_status_line("[PROBE] ERROR code=BACKEND_FAILED exit=1 host=%s phase=initialize",options->host);return 1; }
    probe_set_secret(options->password);
    ap2_io_status_line("[PROBE] TARGET host=%s port=%d", options->host, options->port);
    ap2_io_status_line("[PROBE] PASSWORD_PROVIDED value=%s", options->password && *options->password ? "true" : "false");
    bool accepted = ap2_native_connect(p);
    int result = accepted ? 0 : 1;
    if (p->auth_method)
        ap2_io_status_line("[PROBE] AUTH_METHOD value=%s", p->auth_method);
    if (!accepted) {
        ap2_io_status_line("[PROBE] FAILED phase=%s http=%d detail=%s", p->phase, p->connect_http_status, p->connect_detail);
        LOG_ERROR("[PROBE] local errno=%d (%s)", errno, strerror(errno));
    } else {
        if (p->event_required && p->events_sock < 0) {
            ap2_io_status_line("[PROBE] EVENTS_FAILED");
            result = 1;
        }
        if (result == 0 && !probe_events_start(p)) result = 1;
        if (result == 0 && (options->play_tone || options->pcm_file || options->pcm_stdin)) {
            if (!probe_audio_setup(p)) {
                ap2_io_status_line("[PROBE] FAILED phase=%s http=%d detail=%s", p->phase, p->connect_http_status, p->connect_detail);
                result = 1;
            } else if (!probe_volume_start(p,options)) {
                ap2_io_status_line("[PROBE] VOLUME_CONTROL_FAILED");result=1;
            } else if (!(options->pcm_stdin ? probe_send_live(p, options) : probe_send_audio(p, options))) result = 1;
        }
        /* The default command still tests only a control session. */
        for (int elapsed = 0; result == 0 && elapsed < options->hold_seconds; elapsed += 2) {
            Sleep(2000);
            uint8_t *response = NULL; int length = 0;
            int status = ap2_rtsp_send(p, "GET", "/info", NULL, 0, NULL, &response, &length);
            free(response);
            if (status != 200) {
                ap2_report_failed_exchange(p, "encrypted hold GET /info", status);
                ap2_io_status_line("[PROBE] HOLD_FAILED http=%d", status);
                result = 1;
            }
        }
        if (!probe_events_stop(p)) result = 1;
        if (result == 0) ap2_io_status_line("[PROBE] CONTROL_HOLD_OK seconds=%d", options->hold_seconds);
        if (p->sock_fd >= 0 && !atomic_load(&p->rtsp_dead)) {
            uint8_t *response = NULL; int length = 0;
            int status = ap2_rtsp_send(p, "TEARDOWN", p->session_url, NULL, 0, NULL, &response, &length);
            free(response);
            ap2_io_status_line("[PROBE] TEARDOWN http=%d", status);
            if (status != 200) result = 1;
        }
    }
    probe_tone_feedback_stop(p);
    if (!probe_events_stop(p)) result = 1;
    if (result) result=probe_failure_exit(p);
    probe_context_destroy(p);
    probe_set_secret(NULL);
    ap2_io_status_line("[PROBE] RESULT value=%s", result == 0 ? ((options->play_tone || options->pcm_file || options->pcm_stdin) ? "audio-transport-ok" : "session-ok") : "failed");
    return result;
}

static int probe_run_stereo(const probe_options *options) {
    probe_options other=*options;
    other.host=options->peer_host; other.name=options->peer_name;
    other.txt=options->peer_txt; other.port=options->peer_port;
    other.identity=options->peer_identity; other.active_remote=options->peer_active_remote;
    other.volume_control_port=options->peer_volume_control_port;
    other.peer_host=NULL;
    struct ap2cl_s *p=probe_context_create(options), *q=probe_context_create(&other);
    int result=1;
    probe_set_secret(options->password);
    if (!p || !q) { ap2_io_status_line("[PROBE] ERROR code=BACKEND_FAILED exit=1 host=%s phase=initialize",options->host);goto cleanup; }
    struct ap2cl_s *members[2]={p,q};
    const probe_options *settings[2]={options,&other};
    for (int i=0;i<2;++i) {
        struct ap2cl_s *m=members[i];
        if (i) {
            m->ptp=p->ptp; m->ptp_borrowed=true;
            strcpy(m->group_uuid,p->group_uuid);
            /* The primary has already proved this group's user password.
             * Do not send another deliberately wrong fixed-PIN proof to the
             * peer: HomePod may rate-limit the immediate password retry. */
            if (p->auth_method && !strcmp(p->auth_method,"password") &&
                p->password && *p->password) {
                m->password=p->password;
                ap2_io_status_line("[PROBE] PASSWORD_REUSED host=%s source=authenticated-peer",
                    m->device.address);
            }
        }
        if (i && options->peer_password_first && !(m->password && *m->password)) {
            const char *secret=m->request_password(m->device.address,m->device.name,m->password_arg);
            if (!secret || !*secret) { ap2_io_status_line("[PROBE] ERROR code=PASSWORD_REQUIRED exit=11 host=%s phase=pairing",m->device.address);result=11;goto cleanup; }
            m->password=(char *)secret;
            probe_set_secret(secret);
        }
        ap2_io_status_line("[PROBE] GROUP_CONNECT member=%d host=%s",i,m->device.address);
        if (!ap2_native_connect(m) || !m->use_ptp ||
            (m->event_required && m->events_sock<0) || !probe_events_start(m) ||
            !probe_audio_setup(m) || !probe_volume_start(m,settings[i])) {
            ap2_io_status_line("[PROBE] GROUP_FAILED member=%d host=%s phase=%s http=%d detail=%s",
                i,m->device.address,m->phase,m->connect_http_status,m->connect_detail);
            result=probe_failure_exit(m);
            goto cleanup;
        }
        if (!probe_feedback_start(m)) goto cleanup;
    }
    int lead=p->lead_ms>q->lead_ms?p->lead_ms:q->lead_ms;
    for (int i=0;i<2;++i) {
        struct ap2cl_s *m=members[i];
        if (m->dev_latency_max && MS2TS(lead,44100)>m->dev_latency_max) {
            ap2_io_status_line("[PROBE] GROUP_FAILED reason=incompatible-latency-window"); goto cleanup;
        }
        m->lead_ms=lead;
    }
    struct sockaddr_in local; int local_len=sizeof(local); char our_addr[INET_ADDRSTRLEN];
    if (getsockname(p->sock_fd,(struct sockaddr *)&local,&local_len) ||
        !inet_ntop(AF_INET,&local.sin_addr,our_addr,sizeof(our_addr))) goto cleanup;
    const char *peers[3]={p->device.address,q->device.address,our_addr};
    ap2_ptp_set_peers(p->ptp,peers,3);
    for (int i=0;i<2;++i) {
        ap2_pl_node *array=ap2_pl_array();
        for (int j=0;j<3;++j) ap2_pl_array_append(array,ap2_pl_string(peers[j]));
        uint8_t *body=NULL,*reply=NULL;int reply_len=0;
        int size=ap2_pl_serialize(array,&body);ap2_pl_free(array);
        int status=size>0 ? ap2_rtsp_send(members[i],"SETPEERS",members[i]->session_url,body,size,
            "application/x-apple-binary-plist",&reply,&reply_len):0;
        free(body);free(reply);
        if (status!=200) { ap2_io_status_line("[PROBE] GROUP_FAILED member=%d phase=setpeers http=%d",i,status);goto cleanup; }
    }
    p->group_peer=q;
    ap2_io_status_line("[PROBE] GROUP_READY members=2 shared_ptp=true lead_ms=%d",lead);
    result=probe_send_live(p,options)?0:1;
cleanup:
    probe_tone_feedback_stop(p);probe_tone_feedback_stop(q);
    for (int i=0;i<2;++i) {
        struct ap2cl_s *m=i?q:p;
        if (!m) continue;
        if (!probe_events_stop(m) && result==0) result=1;
        if (m->rtsp_established && !atomic_load(&m->rtsp_dead)) {
            uint8_t *reply=NULL;int length=0;
            int status=ap2_rtsp_send(m,"TEARDOWN",m->session_url,NULL,0,NULL,&reply,&length);free(reply);
            ap2_io_status_line("[PROBE] GROUP_TEARDOWN member=%d host=%s http=%d",i,m->device.address,status);
            if (status!=200 && result==0) result=1;
        }
    }
    if (result==1) ap2_io_status_line("[PROBE] ERROR code=BACKEND_FAILED exit=1 host=%s phase=group",options->host);
    /* Borrower must be destroyed before the one owning the shared engine. */
    probe_context_destroy(q);probe_context_destroy(p);
    probe_set_secret(NULL);
    ap2_io_status_line("[PROBE] RESULT value=%s",result?"failed":"audio-transport-ok");
    return result;
}

