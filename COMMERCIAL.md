# Commercial licensing

VeridianOS's kernel, system servers and Linux runtime are licensed under GPL-3.0-or-later with the VeridianOS user-space exception (see [LICENSING.md](LICENSING.md)). The GPL requires anyone who distributes modified versions of them to publish the source of those modifications, and, for devices, to let owners install modified versions.

If those terms do not fit your product, the maintainer offers **commercial licences** for these parts. Some cases where one is needed:
- a closed driver or system service built with VeridianOS's driver and server framework (the server and driver libraries, the driver SDK, the ring helpers, the server side of the interface definitions), which is GPL-3.0-or-later;
- certainty for any closed driver, system server or other System Component, which the user-space exception does not cover however it is built;
- a device or product built on a modified VeridianOS kernel or servers whose changes cannot be published;
- a locked-down device that cannot allow modified versions to be installed;
- combining VeridianOS kernel code with code under terms incompatible with the GPL;
- support, warranty or indemnity terms the public licences do not give.

Commercial licences are agreed case by case.

What does **not** need one:
- running applications on VeridianOS, including unmodified Linux programs;
- running a Client Program, as the user-space exception defines it: one that is not a System Component (no device capabilities, no serving of VeridianOS protocols, no system services, no authority over the system or other programs) and uses VeridianOS only through its system calls, IPC as a client, or its client libraries; ordinary daemons serving their own protocols, such as databases and web servers, are Client Programs;
- running Linux programs written for Linux, unmodified, whatever they do;
- using the permissively licensed parts (the ABI, libc, crates, tools) under MIT or Apache-2.0.

**Contact:** open an issue titled "Commercial licensing enquiry" at https://github.com/doublegate/VeridianOS/issues (it can be brief; details are then taken off the public tracker), or use the contact details on the maintainer's GitHub profile, https://github.com/doublegate.

The maintainer can offer these licences because every contribution is made under the [Contributor Licence Agreement](CLA.md), which grants the right to license contributions on other terms.
