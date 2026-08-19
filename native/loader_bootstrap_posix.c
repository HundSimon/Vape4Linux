#include "loader_bootstrap.h"

#include <stdio.h>

int vape_loader_bootstrap_initialize(void) {
    return 1;
}

const char *vape_loader_access_token(void) {
    return "0";
}

int vape_loader_bootstrap_failed(void) {
    return 0;
}

void vape_loader_report_progress(int step) {
    (void)step;
}

void vape_loader_report_completed(void) {
}

void vape_loader_report_failure(const char *message) {
    if (message != NULL) {
        fprintf(stderr, "Vape421 agent: %s\n", message);
    }
}

void vape_loader_bootstrap_clear(void) {
}
