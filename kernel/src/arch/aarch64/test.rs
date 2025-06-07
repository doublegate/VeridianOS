// Simple test function

#[no_mangle]
pub unsafe extern "C" fn test_function() {
    let uart = 0x0900_0000 as *mut u8;
    core::ptr::write_volatile(uart, b'F');
}
