// libveridian_dltest.so -- the shared object musl_dynamic_test loads with
// dlopen (ADR 0010). It throws and catches a C++ exception inside itself, so
// loading it also exercises unwinding through a library mapped at run time.

#include <stdexcept>
#include <string>

extern "C" int veridian_dltest(int x)
{
    try {
        throw std::runtime_error(std::to_string(x));
    } catch (const std::exception &e) {
        return static_cast<int>(std::string(e.what()).size());
    }
}
