/* Only a well-formed M4 authentication rejection identifies a rejected SRP
 * secret. HTTP authorization failures and M2 policy failures are not evidence
 * that an AirPlay password would help. Never prompt on a server-proof mismatch. */
static void probe_classify_pair_failure(struct ap2cl_s *p) {
    const ap2_hap_error_t *err=&p->last_pair_error;
    if (err->result!=AP2_HAP_ERR_AUTH) return;
    if (err->tlv_error==3) p->probe_error=12;
    else if (err->tlv_error==5) p->probe_error=16;
    else if (err->tlv_error==4) p->probe_error=17;
    else if (err->tlv_error==6) p->probe_error=18;
    else if (err->tlv_error==7) p->probe_error=19;
    else if (err->http_status==470) p->probe_error=14;
    else if (err->http_status==401 || err->http_status==403) p->probe_error=15;
    else if (err->pair_state==4 && err->tlv_error==2)
        p->probe_error=p->password && *p->password ? 10 : 11;
    else p->probe_error=13;
    p->connect_error=AP2_CONNECT_ERROR_AUTH_FAILED;
}
static bool ap2_native_pair(struct ap2cl_s *p) {
    bool automatic=!(p->password && *p->password) && !p->auth_credentials;
    ap2_io_status_line("[PROBE] AUTH_ATTEMPT host=%s mode=%s",p->device.address,
        automatic?"automatic":p->auth_credentials?"credentials":"password");
    if (probe_pair_once(p)) {
        ap2_io_status_line("[PROBE] AUTH_COMPLETE host=%s method=%s",p->device.address,p->auth_method);
        return true;
    }
    probe_classify_pair_failure(p);
    if (!automatic || p->probe_error!=11 || !p->request_password) return false;
    ap2_io_status_line("[PROBE] AUTH_CHALLENGE host=%s code=PASSWORD_REQUIRED",p->device.address);
    const char *password=p->request_password(p->device.address,p->device.name,p->password_arg);
    if (!password || !*password) return false;
    p->password=(char *)password;
    probe_set_secret(password);
    p->probe_error=0;
    p->connect_error=AP2_CONNECT_ERROR_NONE;
    if (!ap2_native_reset_connection(p)) return false;
    p->phase="pairing";
    memset(&p->last_pair_error,0,sizeof(p->last_pair_error));
    ap2_io_status_line("[PROBE] AUTH_ATTEMPT host=%s mode=password",p->device.address);
    if (probe_pair_once(p)) {
        ap2_io_status_line("[PROBE] AUTH_COMPLETE host=%s method=%s",p->device.address,p->auth_method);
        return true;
    }
    probe_classify_pair_failure(p);
    return false;
}
