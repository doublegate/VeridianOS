// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item expanded affix "><a href="introduction.html">Introduction</a></li><li class="chapter-item expanded affix "><li class="part-title">Getting Started</li><li class="chapter-item expanded "><a href="getting-started/prerequisites.html"><strong aria-hidden="true">1.</strong> Prerequisites</a></li><li class="chapter-item expanded "><a href="getting-started/building.html"><strong aria-hidden="true">2.</strong> Building VeridianOS</a></li><li class="chapter-item expanded "><a href="getting-started/running.html"><strong aria-hidden="true">3.</strong> Running in QEMU</a></li><li class="chapter-item expanded "><a href="getting-started/dev-setup.html"><strong aria-hidden="true">4.</strong> Development Setup</a></li><li class="chapter-item expanded affix "><li class="part-title">Architecture</li><li class="chapter-item expanded "><a href="architecture/overview.html"><strong aria-hidden="true">5.</strong> Overview</a></li><li class="chapter-item expanded "><a href="architecture/microkernel.html"><strong aria-hidden="true">6.</strong> Microkernel Design</a></li><li class="chapter-item expanded "><a href="architecture/memory.html"><strong aria-hidden="true">7.</strong> Memory Management</a></li><li class="chapter-item expanded "><a href="architecture/processes.html"><strong aria-hidden="true">8.</strong> Process Management</a></li><li class="chapter-item expanded "><a href="architecture/ipc.html"><strong aria-hidden="true">9.</strong> Inter-Process Communication</a></li><li class="chapter-item expanded "><a href="architecture/capabilities.html"><strong aria-hidden="true">10.</strong> Capability System</a></li><li class="chapter-item expanded "><a href="architecture/drivers.html"><strong aria-hidden="true">11.</strong> Device Drivers</a></li><li class="chapter-item expanded affix "><li class="part-title">Development Guide</li><li class="chapter-item expanded "><a href="development/organization.html"><strong aria-hidden="true">12.</strong> Code Organization</a></li><li class="chapter-item expanded "><a href="development/standards.html"><strong aria-hidden="true">13.</strong> Coding Standards</a></li><li class="chapter-item expanded "><a href="development/testing.html"><strong aria-hidden="true">14.</strong> Testing</a></li><li class="chapter-item expanded "><a href="development/debugging.html"><strong aria-hidden="true">15.</strong> Debugging</a></li><li class="chapter-item expanded "><a href="development/performance.html"><strong aria-hidden="true">16.</strong> Performance</a></li><li class="chapter-item expanded "><a href="development/security.html"><strong aria-hidden="true">17.</strong> Security</a></li><li class="chapter-item expanded affix "><li class="part-title">Platform Support</li><li class="chapter-item expanded "><a href="platforms/x86_64.html"><strong aria-hidden="true">18.</strong> x86_64</a></li><li class="chapter-item expanded "><a href="platforms/aarch64.html"><strong aria-hidden="true">19.</strong> AArch64</a></li><li class="chapter-item expanded "><a href="platforms/riscv.html"><strong aria-hidden="true">20.</strong> RISC-V</a></li><li class="chapter-item expanded affix "><li class="part-title">Kernel Subsystems</li><li class="chapter-item expanded "><a href="kernel/boot.html"><strong aria-hidden="true">21.</strong> Boot Process</a></li><li class="chapter-item expanded "><a href="kernel/allocator.html"><strong aria-hidden="true">22.</strong> Memory Allocator</a></li><li class="chapter-item expanded "><a href="kernel/scheduler.html"><strong aria-hidden="true">23.</strong> Scheduler</a></li><li class="chapter-item expanded "><a href="kernel/syscalls.html"><strong aria-hidden="true">24.</strong> System Calls</a></li><li class="chapter-item expanded "><a href="kernel/interrupts.html"><strong aria-hidden="true">25.</strong> Interrupt Handling</a></li><li class="chapter-item expanded affix "><li class="part-title">API Reference</li><li class="chapter-item expanded "><a href="api/kernel.html"><strong aria-hidden="true">26.</strong> Kernel API</a></li><li class="chapter-item expanded "><a href="api/syscalls.html"><strong aria-hidden="true">27.</strong> System Call API</a></li><li class="chapter-item expanded "><a href="api/drivers.html"><strong aria-hidden="true">28.</strong> Driver API</a></li><li class="chapter-item expanded affix "><li class="part-title">Contributing</li><li class="chapter-item expanded "><a href="contributing/how-to.html"><strong aria-hidden="true">29.</strong> How to Contribute</a></li><li class="chapter-item expanded "><a href="contributing/review.html"><strong aria-hidden="true">30.</strong> Code Review Process</a></li><li class="chapter-item expanded "><a href="contributing/docs.html"><strong aria-hidden="true">31.</strong> Documentation</a></li><li class="chapter-item expanded affix "><li class="part-title">Design Documents</li><li class="chapter-item expanded "><a href="design/memory-allocator.html"><strong aria-hidden="true">32.</strong> Memory Allocator Design</a></li><li class="chapter-item expanded "><a href="design/ipc-system.html"><strong aria-hidden="true">33.</strong> IPC System Design</a></li><li class="chapter-item expanded "><a href="design/scheduler.html"><strong aria-hidden="true">34.</strong> Scheduler Design</a></li><li class="chapter-item expanded "><a href="design/capability-system.html"><strong aria-hidden="true">35.</strong> Capability System Design</a></li><li class="chapter-item expanded affix "><li class="part-title">Development Phases</li><li class="chapter-item expanded "><a href="phases/phase0-foundation.html"><strong aria-hidden="true">36.</strong> Phase 0: Foundation</a></li><li class="chapter-item expanded "><a href="phases/phase1-microkernel.html"><strong aria-hidden="true">37.</strong> Phase 1: Microkernel Core</a></li><li class="chapter-item expanded "><a href="phases/phase2-userspace.html"><strong aria-hidden="true">38.</strong> Phase 2: User Space</a></li><li class="chapter-item expanded "><a href="phases/phase3-security.html"><strong aria-hidden="true">39.</strong> Phase 3: Security</a></li><li class="chapter-item expanded "><a href="phases/phase4-packages.html"><strong aria-hidden="true">40.</strong> Phase 4: Package Ecosystem</a></li><li class="chapter-item expanded "><a href="phases/phase5-performance.html"><strong aria-hidden="true">41.</strong> Phase 5: Performance</a></li><li class="chapter-item expanded "><a href="phases/phase6-advanced.html"><strong aria-hidden="true">42.</strong> Phase 6: Advanced Features</a></li><li class="chapter-item expanded affix "><li class="part-title">Project Information</li><li class="chapter-item expanded "><a href="project/status.html"><strong aria-hidden="true">43.</strong> Project Status</a></li><li class="chapter-item expanded "><a href="project/roadmap.html"><strong aria-hidden="true">44.</strong> Roadmap</a></li><li class="chapter-item expanded "><a href="project/faq.html"><strong aria-hidden="true">45.</strong> FAQ</a></li><li class="chapter-item expanded "><a href="project/troubleshooting.html"><strong aria-hidden="true">46.</strong> Troubleshooting</a></li><li class="chapter-item expanded "><a href="project/performance-baselines.html"><strong aria-hidden="true">47.</strong> Performance Baselines</a></li><li class="chapter-item expanded affix "><li class="part-title">Advanced Topics</li><li class="chapter-item expanded "><a href="advanced/software-porting.html"><strong aria-hidden="true">48.</strong> Software Porting Guide</a></li><li class="chapter-item expanded "><a href="advanced/compiler-toolchain.html"><strong aria-hidden="true">49.</strong> Compiler Toolchain</a></li><li class="chapter-item expanded "><a href="advanced/formal-verification.html"><strong aria-hidden="true">50.</strong> Formal Verification</a></li><li class="chapter-item expanded affix "><li class="spacer"></li><li class="chapter-item expanded affix "><a href="changelog.html">Changelog</a></li><li class="chapter-item expanded affix "><a href="security.html">Security Policy</a></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString().split("#")[0].split("?")[0];
        if (current_page.endsWith("/")) {
            current_page += "index.html";
        }
        var links = Array.prototype.slice.call(this.querySelectorAll("a"));
        var l = links.length;
        for (var i = 0; i < l; ++i) {
            var link = links[i];
            var href = link.getAttribute("href");
            if (href && !href.startsWith("#") && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The "index" page is supposed to alias the first chapter in the book.
            if (link.href === current_page || (i === 0 && path_to_root === "" && current_page.endsWith("/index.html"))) {
                link.classList.add("active");
                var parent = link.parentElement;
                if (parent && parent.classList.contains("chapter-item")) {
                    parent.classList.add("expanded");
                }
                while (parent) {
                    if (parent.tagName === "LI" && parent.previousElementSibling) {
                        if (parent.previousElementSibling.classList.contains("chapter-item")) {
                            parent.previousElementSibling.classList.add("expanded");
                        }
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', function(e) {
            if (e.target.tagName === 'A') {
                sessionStorage.setItem('sidebar-scroll', this.scrollTop);
            }
        }, { passive: true });
        var sidebarScrollTop = sessionStorage.getItem('sidebar-scroll');
        sessionStorage.removeItem('sidebar-scroll');
        if (sidebarScrollTop) {
            // preserve sidebar scroll position when navigating via links within sidebar
            this.scrollTop = sidebarScrollTop;
        } else {
            // scroll sidebar to current active section when navigating via "next/previous chapter" buttons
            var activeSection = document.querySelector('#sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        var sidebarAnchorToggles = document.querySelectorAll('#sidebar a.toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(function (el) {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define("mdbook-sidebar-scrollbox", MDBookSidebarScrollbox);
