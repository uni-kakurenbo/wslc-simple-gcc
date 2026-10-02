#include <stdio.h>

int main(int argc, char **argv) {
    printf("Hello C! GCC %s; __STDC_VERSION__=%ld\n",
           __VERSION__, (long)__STDC_VERSION__);
    for (int i = 1; i < argc; ++i) {
        printf("argv[%d]=<%s>\n", i, argv[i]);
    }
    return 0;
}
