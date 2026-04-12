import time
import subprocess
import os

try:
    import pyautogui
    import pyperclip
except ImportError:
    print("Dependencies not found, please ensure python3-pyautogui and python3-pyperclip are installed.")
    os._exit(1)

def run_test(test_name, input_action, expected):
    print(f"\n=== Starting Test: {test_name} ===")
    
    print("Switching to fcitx (ctrl+space)...")
    pyautogui.hotkey('ctrl', 'space')
    time.sleep(0.5)

    # Execute the custom input action
    input_action()

    print("Switching back to English (ctrl+space)...")
    pyautogui.hotkey('ctrl', 'space')
    time.sleep(0.5)

    print("Selecting all text (ctrl+a) and cutting (ctrl+x)...")
    # Using ctrl+a then ctrl+x will both copy to clipboard and clear the mousepad editor for the next test
    pyautogui.hotkey('ctrl', 'a')
    time.sleep(0.1)
    pyautogui.hotkey('ctrl', 'x')
    time.sleep(0.5)

    print("Checking clipboard content via pyperclip...")
    clipboard_content = pyperclip.paste()

    print("-" * 57)
    if clipboard_content == expected:
        print(f"✅ TEST PASSED: Clipboard successfully verified as '{expected}'!")
    else:
        print(f"❌ TEST FAILED: Expected '{expected}', but clipboard contains: '{clipboard_content}'")
    print("-" * 57)

    # Empty the clipboard
    pyperclip.copy('')
    print("Clipboard cleared.")

def test_nihao():
    def action():
        print("Typing 'nihao'...")
        pyautogui.write('nihao', interval=0.05)
        pyautogui.press('space')
        time.sleep(0.5)
    run_test("Basic 'nihao' commit", action, "你好")

def test_nihaoshijie_1():
    def action():
        print("Typing 'nihaoshijie1' + space...")
        pyautogui.write('nihaoshijie', interval=0.05)
        time.sleep(0.1)
        pyautogui.write('1', interval=0.05)
        time.sleep(0.1)
        pyautogui.press('space')
        time.sleep(0.5)
    run_test("Multi-round 'nihaoshijie' + '1' + space", action, "你好世界 ")

def test_nihaoshijie_2_partial():
    def action():
        print("Typing 'nihaoshijie2,'...")
        pyautogui.write('nihaoshijie', interval=0.05)
        time.sleep(0.1)
        pyautogui.write('2,', interval=0.05)
        time.sleep(0.5)
    run_test("Multi-round 'nihaoshijie' + '2' + comma", action, "你好时节，")

def test_uppercase_upan():
    def action():
        print("Typing 'Upan' + space...")
        # Since 'U' is uppercase, PyAutoGUI will automatically send Shift+u
        pyautogui.write('Upan', interval=0.05)
        time.sleep(0.1)
        pyautogui.press('space')
        time.sleep(0.5)
    run_test("Uppercase 'Upan'", action, "U盘")

def test_punctuations_and_numbers():
    def action():
        print("Typing '123,'...")
        pyautogui.write('123,', interval=0.05)
        time.sleep(0.5)
    run_test("Punctuations and numbers '123,'", action, "123，")

def test_complex_time_format():
    def action():
        print("Typing '23:59'...")
        pyautogui.write('23:59', interval=0.05)
        time.sleep(0.5)
    run_test("Complex time format '23:59'", action, "23：59")

def test_meizi_commit():
    def action():
        print("Typing 'mizi1'...")
        pyautogui.write('mizi', interval=0.05)
        time.sleep(0.1)
        pyautogui.write('1', interval=0.05)
        time.sleep(0.5)
    run_test("Meizi candidate jumping 'mizi1'", action, "糜子")

def test_mp3():
    def action():
        print("Typing 'mp3'...")
        pyautogui.write('mp3', interval=0.05)
        time.sleep(0.5)
    run_test("Auto English 'mp3'", action, "MP3")

def test_auto_brace():
    def action():
        print("Typing '(a'...")
        pyautogui.write('(a', interval=0.05)
        time.sleep(0.5)
        pyautogui.press('space')
        time.sleep(0.5)
    run_test("Auto brace '(a'", action, "（啊）")

def test_v_mode():
    def action():
        print("Typing 'v1.1a'...")
        pyautogui.write('v1.1a', interval=0.05)
        time.sleep(0.5)
    run_test("V mode correct 'v1.1a'", action, "一元一角")

def test_v_mode_wrong():
    def action():
        print("Typing 'v1.11'...")
        pyautogui.write('v1.11', interval=0.05)
        time.sleep(0.5)
    run_test("V mode wrong 'v1.11'", action, "")

def test_v_mode_fake():
    def action():
        print("Typing 'vase1'...")
        pyautogui.write('vase1', interval=0.05)
        time.sleep(0.5)
    run_test("Not V mode but 'v' prefix 'vase1'", action, "vase")

def test_v_mode_fake_wrong():
    def action():
        print("Typing 'vasea'...")
        pyautogui.write('vasea', interval=0.05)
        time.sleep(0.5)
    run_test("Not V mode but 'v' prefix 'vasea'", action, "")

def main():
    # Ensure window is active before starting tests
    subprocess.run('xdotool search --sync --onlyvisible --class "mousepad" windowactivate', shell=True)
    time.sleep(0.5)

    test_nihao()
    test_nihaoshijie_1()
    test_nihaoshijie_2_partial()
    test_uppercase_upan()
    test_punctuations_and_numbers()
    test_complex_time_format()
    test_meizi_commit()
    test_mp3()
    # test_auto_brace()
    test_v_mode()
    test_v_mode_wrong()
    test_v_mode_fake()
    test_v_mode_fake_wrong()

if __name__ == "__main__":
    main()
