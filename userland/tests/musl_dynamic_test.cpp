// musl_dynamic_test -- a dynamically linked C++ program (ADR 0010).
//
// Built against the shared musl, libstdc++ and libgcc_s, so the kernel
// starts it in musl's dynamic loader (/lib/ld-musl-x86_64.so.1), which maps
// the shared libraries and relocates everything before main. It then
// exercises what depends on that working: C++ exceptions (unwinding through
// libgcc_s), threads with a thread_local destructor (TLS in a dynamically
// linked program), and dlopen of a C++ shared object that throws and catches
// inside itself.
//
// musl_runtime_test runs it and checks its output and exit status
// ("musl_dynamic_program"). Prints one result line; exit 0 if all is well.

#include <dlfcn.h>
#include <sys/auxv.h>

#include <cstdio>
#include <stdexcept>
#include <string>
#include <thread>

namespace {
bool tls_destroyed = false;

struct Tls {
    ~Tls() { tls_destroyed = true; }
};
thread_local Tls tls;
}  // namespace

int main()
{
    // Started by the interpreter: the kernel passes its load bias.
    const bool interpreted = getauxval(AT_BASE) != 0;

    std::string what;
    try {
        throw std::runtime_error("thrown");
    } catch (const std::exception &e) {
        what = e.what();
    }

    std::thread worker([] { (void)tls; });
    worker.join();

    // Found in /usr/lib, on musl's default library path.
    void *lib = dlopen("libveridian_dltest.so", RTLD_NOW);
    auto fn = lib ? reinterpret_cast<int (*)(int)>(dlsym(lib, "veridian_dltest")) : nullptr;
    const int loaded = fn ? fn(12345) : -1;

    const bool ok = interpreted && what == "thrown" && tls_destroyed && loaded == 5;
    std::printf("dynamic: interpreted=%d exception=%s tls_dtor=%d dlopen=%d error=%s %s\n",
                interpreted, what.c_str(), tls_destroyed, loaded, lib ? "-" : dlerror(),
                ok ? "OK" : "FAIL");
    return ok ? 0 : 1;
}
