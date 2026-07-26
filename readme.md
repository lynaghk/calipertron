# Calipertron

A lil' exploration in understanding how electronic calipers work.

See [full background and results](https://kevinlynagh.com/calipertron/).

<video src="https://github.com/user-attachments/assets/29552f66-8df2-4e82-a5d7-7be6bd64245d" controls="true" loop="true" autoplay="true">
    <p>A demo video showing a sliding caliper and position values updating in real time.</p>
</video>

All of the work in this repo was done with the [v1.1 PCB designed by Mitko](https://github.com/MitkoDyakov/Calipatron/tree/444c72c3e81eab0a2e7ee198f5574062dc1fc510/Hardware/V1.1).
See also his [DIY Digital Caliper](https://hackaday.io/project/194778-diy-digital-caliper) Hackday.io project page.

## install

First [install Rust](https://www.rust-lang.org/tools/install), then:

    rustup target add thumbv7m-none-eabi
    cargo install probe-rs-tools

For Python analysis stuff, [install UV](https://github.com/astral-sh/uv?tab=readme-ov-file#installation).
then run

    cd analysis/
    uv sync


## firmware/

Rust firmware for v1.1 PCB based on the Embassy framework.

Build and flash via STLink:

    cargo run --release --bin local

Attach to running firmware:

    probe-rs attach --chip STM32F103C8 target/thumbv7m-none-eabi/release/local
    
I did all of the development using Rust 1.81 on an M1 Macbook air running MacOS 12.7.6.


## frontend/

Parameter sweep:

    cargo run --release --bin parameter_sweep
    

## Log

### 2026 Jul 26 - Have Fable throw together a few ideas

I collected some raw ADC data and fed it into Anthropic's Fable 5 and had it write up some improvements.
Notably:

- Take into account the way the frequency divider works so our ADC sampling lines up exactly with our PDM drive signal
- Exponential moving average phasors rather than phase (this prevents issues around +/- π)
- Use a Hann window rather than rectangular window to reduce noise

Now when the slide isn't moving the reading remains stable, and precision I get is about ~0.1mm while moving the slide

### Nov 2 - Initial public release, hurray!

### Nov 1 - Do proper grid search of params.

Make frontend parameter_sweep binary robust to the device freezing and needing to be restarted.


### Oct 23 - Do some design simluations to better understand PDM and sampling

Added a new design_simulations notebook to try and understand whether an analog low-pass filter is necessary to smooth the reflected signal before reading by the ADC, or if all of that will wash out in software.
Seems like it's the latter: Even when we're at the slowest sampling rate we're still getting phase error of just e-14
Interestingly this is less than the e-12 error of the longest sampling rate --- I guess that's acting as a low pass filter itself and helping a bit.
(And/or all of this washes out when correlating across 10 full cycles)


### Oct 18/19 - frequency scan; phase via cross correlation

Haven't had much luck using Z3 to find optimal FFT settings, search space as I set it up is probably too bug.

I'm likely going about this the wrong way, anyway --- it's probably much better to just pick the PDM signal frequency to be something that's not a multiple of 50 or 60 Hz line noise, then extract the phase by cross-correlating with the known frequency

Nice overview of PDM theory https://tomverbeure.github.io/2020/10/04/PDM-Microphones-and-Sigma-Delta-Conversion.html

---

How many periods of the emitted signal should we capture?
Also, does it matter if the number of samples line up with the signal?
I don't think it really does, since we know the signal frequency.

For simplicity, lets just record a fixed number of samples for each emission frequency.


### Sept 11

Doing FFT on stm32f103:

44.3ms to sample
138.0ms to sample + 2048 FFT.
141.0ms to sample + 2048 FFT + arctan phase calculation for 5 bins.

Presumably takes forever because we don't have a floating point unit.
For better performance we could probably do the DFT only for the target bin.

Looks like we're split across bins 32 and 33. Need to tweak signal and sampling rates so all energy is centered in one bin.

30: 172994200.0 -0.6752128
31: 440644960.0 -0.7401424
32: 3207128000.0    -0.7714465
33: 6121130500.0    2.3279614
34: 512542240.0 2.2985573

over 25 measurements without moving the slide, bin 33 phase is 2.33 +/- 0.045.

Would be nice if it's a bit steadier.


### Sept 9

Got new PCB scale from Dimitar with correct measurements.
Added new firmware to do "one shot" recording of 32 cycles, rather than trying to do it continously and stream to usb (the phase offset of the initial start was inconsistent here).

I verified with the python analysis notebook that the results are pretty repeatable.
I.e., multiple measurements at same location give phase offset repeatable to about 0.1 radians.
Moving the scale reliably changes the phase offset.

Should be straightforward to update the notebook to "poll" continuously and then get a live plot to mess with.

### Aug 23/24/25 - signal analysis
Recording coupled signals and trying to decode them in Python.

Sampling rate is driven by ADC sample time (assuming no DMA overrun / drop out for simplicity)

1. System clock (HCLK) = 72 MHz
2. APB2 clock = 72 MHz
3. ADC clock = 72 MHz / 6 = 12 MHz (14 MHz max)

ADC freq (/ (* 18 1e6) 239.5) =>  75.156 kHz
ADC freq (/ (* 18 1e6) 71.5)  => 251.748 kHz

PDM array has 132 entries, so assuming that's one period then the sinusoidal drive signal frequency is
(/ 100000 132.0) => 757.57 Hz
(/  50000 132.0) => 378.79 Hz


---

I'm not sure how to process the captured signal.
It's not a simple PDM because the amplitude isn't binary.

Maybe I should try just emitting a single GPIO signal and seeing if I can reconstruct that first?

Okay, yeah, that definitely looks more like a PDM signal.
So the modulation I'm getting probably really is the desired effect of the signals combining.
I wonder if I should just do a rolling window and calculate variance and then take the minimum of that or something?

---

Verifying the PDM by importing the drive signal into python and analyzing, we match the expected 757.58 drive frequency. Cool.
Though this may be begging the question since the calcs use the drive rate, which is derived assuming the ADC clock stuff above is correct.
Let's see if we can recover it from a single signal

---

If I take a rolling 20-sample window of the high-pass filtered signal, I get a decent looking sine wave.
However, the frequency is 2670 Hz. Where does this come from?


Ahh, it's not really a clean frequency. I can fit 10-ish cycles, but not more than that.
Manually playing with the curve fit I can see it's not lining up nicely.
The spectrogram isn't super clean, but there's a peak much higher than the rest at 2668 Hz, basically the same as we saw above.


----

Hmm, lets look at the coupled signal with just a single drive PDM.
With 2000Hz PDM cutoff, there's a strong peak at 1385 Hz.

Maybe the drive timer is actually off?
After all, Embassy is probably doing some rounding or whatever to set the frequency. But by almost 2x? That seems pretty bad.

Let's try with 10kHz timer (PDM drive rate)

no real luck. though maybe I'm removing the signal with the low pass filter that I used to eliminate the line noise?
If I look at max power without low pass filter, it's around 84 Hz which is close to the 76 Hz theoretical.

I should crank up the PDM tick so that the generated signal is well above 50 Hz.

Going back to the 100kHz tick (757 Hz drive)

the unfiltered peak is 84 Hz still (???)
AHHH, is that microcontroller noise? Nah, that's 72 MHz.
Hmm, power grid here should be about 50 Hz.

Okay, did another recording of just line noise and it's still coming out at 83.92 Hz.
So the ADC sample rate must be off.

Yeah, Claude bullshitted me. Looking at the clock tree it says ADC max is 14 MHz.
let's look at registers with cube debugger
PLLMUL => pll input clock x 9

HPRE = AHB prescaler, SYSCLK not divided
PPRE2 = APB2 prescaler HCLK not divided
ADCPRE = PCLK2 divided by 6

Okay, so assuming we're at max SYSCLK of 72 MHz, that gives ADC clock of 12 MHz.

Adjusting for this, line noise frequency (FFT of raw data) comes out to 58.74 Hz, which is pretty off.

Hmm, maybe that's correlated noise from shorter sample time.
When I use the longest ADC sampling time, the collected data gives 52.61 Hz.
If I software filter out noise above 100 Hz, then it's 50.10 Hz.


Let's collect more driven data using this longer sample time.

Okay! Peak is at 796 Hz. That's close to 757 Hz drive, I guess.

What if I do 50?
Hmm, seeing two peaks, biggest at 1195 Hz.
Not sure how this makes sense, since I'm filtering loss pass with 1000 Hz cutoff

If I tweak the filtering, I can recover 737 Hz.

GPIO speed shoudln't be an issue, it's set to slowest but that's 2 MHz.

Taking line noise again with longest sample time, FFT shows peak at 52.61 Hz again.

I don't think it's a hardware thing, I measured on bluepill too.
Holding the cable really helps.

---

I measured line noise with my scope, it's definietly just 50 Hz.
MCU clock doesn't seem to be a problem, since if I emit a 1kHz signal on the GPIO the scope measures it as 1.00 kHz.
Emitting 50 Hz signal measures as 50.00 Hz. So MCU clock seems aight.

Possible the issue is related to DMA dropping occasional samples and the larger signal getting out of sync?
If that were the case, I should see freq get "more accurate" with less data.

Nah, changing the offsets and amonut doesn't move it much, still around 52.6 Hz.
Maybe it's roundoff / numerical error on the python FFT impl?
That seems quite unlikely, but not sure what else it might be.

---

Recording the MCU's generated 50Hz square wave with the ADC and analyzing in the computer gives 52.89 Hz FFT.

Ah, found the issue! ADC converstion time is sampling duration + a 12.5 cycle overhead.
ADding that in gives me FFT on generated signal of 50.01 Hz.

---

Now looking at a single PDM waveform ticked out at 100 kHz (implied 757.6 Hz signal), high pass above 500 Hz to remove line-noise and low pass under 1000 Hz I see just a single peak at 758 Hz. Nice!


### Aug 6

Had a chat with Mitko.
Look into "pulse density modulation" for signal generation. Also read color university paper in repo. Mitko will add patent too.

---

Looking more into minimal reproducible example for the hang I've been running into.

It seems like the issue may be resolved by doing a full power reset when flashing new firmware.
So maybe something about the USB bus is getting corrupted in such a way that causes the later hang?

I do need to trigger interrupt when USB disconnects --- right now the USB tasks just hang in wfe.
(discovered this via unplugging and then Ctrl-C probe-run, which listed /Users/dev/.cargo/git/checkouts/embassy-9312dcb0ed774b29/46aa206/embassy-executor/src/arch/cortex_m.rs:106:21)

Managed to catch and debug registers via STM32Cube.
ICSR
ISRPending 1
VectPending 0x024


This vect points to USB stuff, but I didn't see any obvious error flags in the USB registers.

Had another freeze and captured the pending interrupt as 0x01B, which is DMA1_CHANNEL5

// TODO: possible I need to bind
// DMA1_CHANNEL5 => timer::UpdateInterruptHandler<peripherals::TIM1>;


When the app is running fine, though, and I read this register, it fluctuates between:

0x01B
0x01F

and also (when USB is being read)

0x024

I'm not sure how Claude knew 0x024 was USB --- I'd like to see this in a datasheet

the enum values in https://docs.embassy.dev/embassy-stm32/git/stm32f103c8/interrupt/enum.Interrupt.html don't match


---

- if code is mapped into RAM, it's possible stack could clobber it if it's too big.
debuggers
- uses jetbrains integrated debugger in RustRover.
- espressif has onboard debugger over USB
- set breakpoints, watchpoints etc.
- break on interrupt


---

crash still occurs when debugger data cables disconnected


tried adding my own hardfault handler

use cortex_m_rt::{exception, ExceptionFrame};
#[exception]
unsafe fn HardFault(ef: &ExceptionFrame) -> ! {
loop {}
}

but didn't end up there on the freeze. (all registers point to 0x2100_0000

port install gdb +multiarch

break on exception.



### Aug 5 - debugging


https://matrix.to/#/!YoLPkieCYHGzdjUhOK:matrix.org/$dOadSX4X9q9CnQEiKhKyZJr5toI8R8q08fk8qE3VloE?via=matrix.org&via=tchncs.de&via=mozilla.org



14:49:40 $ /Applications/STMicroelectronics/STM32Cube/STM32CubeProgrammer/STM32CubeProgrammer.app/Contents/MacOs/bin/STM32_Programmer_CLI -c port=SWD
-------------------------------------------------------------------
STM32CubeProgrammer v2.17.0
-------------------------------------------------------------------

ST-LINK SN  : 53FF6C064884534937360587
ST-LINK FW  : V2J37S7
Board       : --
Voltage     : 3.24V
SWD freq    : 4000 KHz
Connect mode: Normal
Reset mode  : Software reset
Device ID   : 0x410
Revision ID : Rev X
Device name : STM32F101/F102/F103 Medium-density
Flash size  : 64 KBytes
Device type : MCU
Device CPU  : Cortex-M3
BL Version  : --


I didn't see anything in errata that seemed sus: https://www.st.com/resource/en/errata_sheet/es0340-stm32f101xcde-stm32f103xcde-device-errata-stmicroelectronics.pdf


cargo install cargo-binutils
rustup component add llvm-tools-preview

I can print out assembly of my program now via

    cargo objdump --bin usb_custom --release -- -d --no-show-raw-insn --print-imm-hex

that doesn't help me yet, since I don't have any fault handlers pointing to what went wrong.


### Aug 3 - ADC pickup
DMA ring buffer is working.
the duplicates I was getting earlier is because stop disables the circularity on the channel, but start doesn't restore it.
so if you call stop once, you get screwed forever.

(I should just reset the device if USB ever disconnects)

based on defmt timing, each loop takes about 600--700us.
awaiting USB causes us to overrun DMA, so I may need to have a larger buffer and do that in a separate task.


Ugh. I have no idea what's fucked here.
Even when not trying to send over USB, eventually the ringbuffer freezes somehow and the await loop stops.


ugh, maybe it's bad wiring?
just reflashing and fucking about, no real changes and now it's streaming alnog fine.

    // use embassy_stm32::interrupt;
    // #[interrupt]
    // fn DMA1_CHANNEL1() {
    //     info!("interrupt");
    // }

yeah, very intermittent.
Everything seems to be working fine now. this is infuriating.



### Aug 2 - signal emission and ADC pickup
Claude helped me make a little USB scope visualization.

For the life of me I can't seem to affect the waveform, though.
The average value seems to change on every reset.

There's no pulldown, so I thought it might be a floating voltage.
But trying to clear it with

    {
        let _adc_pin = Output::new(p.PB1, Level::Low, Speed::VeryHigh);
        Timer::after_millis(1000).await;
        // dropping returns to high impedence
    }

doesn't affect things.
average value can be anywhere from 800 to 1400.
Though I guess looking at the schematic the floating is occucring before the MCP6S21 amplifier.


Let's rule out it's not some USB delay thing by generating a cycling counter.

Yeah, USB and egui rendering look superfast, no problems there.

Maybe the ADC isn't actually sampling properly and it's repeating itself?

Yeah, I think that's what's happening. scrolling through the output file, the readings are super identical.


Okay, it's something in embassy ring buffer.
If I have DMA write to a static mut and I just YOLO read from it, the values are definitely change

from looking at https://github.com/embassy-rs/embassy/blob/a2ea2630f648a9a46dc4f9276d9f5eaf256ae893/embassy-stm32/src/adc/ringbuffered_v2.rs#L122 it seems like I'm holding the ring buffer correctly. hmmm.



### July 31 - Rust USB ADC data streaming.
Got this working first by porting Embassy USB CDC example, but on MacOS there's some kind of internal buffer that waits to fill up before any results are printed out of the file.
It takes 10+ seconds so is probably a few MB.

I used Cursor to generate, then had it do a custom USB class protocol thing so I can bypass the CDC_ACM virtual serial port stuff.

It took a bit of coaxing, but I managed to get something working with a custom reader too.

Embassy seems to have nice DMA stuff, but it doesn't seem to support F1-series.
https://github.com/embassy-rs/embassy/pull/3116

I should just give up on this for now and stream naively.





### July 29 - Arudino test

installed arduino-ide_2.3.2_macOS_arm64


followed https://community.st.com/t5/stm32-mcus/how-to-program-and-debug-the-stm32-using-the-arduino-ide/ta-p/608514

add board manager URL in preferences: https://github.com/stm32duino/BoardManagerFiles/raw/main/package_stmicroelectronics_index.json

installed STM32 MCU based boards version 2.8.1

Trying to upload:

    Sketch uses 14896 bytes (45%) of program storage space. Maximum is 32768 bytes.
    Global variables use 1632 bytes (15%) of dynamic memory, leaving 8608 bytes for local variables. Maximum is 10240 bytes.
    STM32CubeProgrammer not found (STM32_Programmer_CLI).
      Please install it or add '<STM32CubeProgrammer path>/bin' to your PATH environment:
      https://www.st.com/en/development-tools/stm32cubeprog.html

Aight, had to install cube stuff but then the arduino upload worked:


    Sketch uses 14896 bytes (45%) of program storage space. Maximum is 32768 bytes.
    Global variables use 1632 bytes (15%) of dynamic memory, leaving 8608 bytes for local variables. Maximum is 10240 bytes.
    Warning: long options not supported due to getopt from FreeBSD usage.
    Selected interface: swd
    -------------------------------------------------------------------
    STM32CubeProgrammer v2.17.0
    -------------------------------------------------------------------

    ST-LINK SN  : 53FF6C064884534937360587
    ST-LINK FW  : V2J37S7
    Board       : --
    Voltage     : 3.25V
    SWD freq    : 4000 KHz
    Connect mode: Under Reset
    Reset mode  : Hardware reset
    Device ID   : 0x410
    Revision ID : Rev X
    Device name : STM32F101/F102/F103 Medium-density
    Flash size  : 64 KBytes
    Device type : MCU
    Device CPU  : Cortex-M3
    BL Version  : --



    Memory Programming ...
    Opening and parsing file: Base.ino.bin
    File          : Base.ino.bin
    Size          : 14.84 KB
    Address       : 0x08000000


    Erasing memory corresponding to segment 0:
    Erasing internal memory sectors [0 14]
    Download in Progress:


    File download complete
    Time elapsed during download operation: 00:00:01.048

    RUNNING Program ...
    Address:      : 0x8000000
    Application is running, Please Hold on...
    Start operation achieved successfully


Was also able to get serial monitor working by setting USB Support Generic Serial CDC.
Man, Arduino is nicer than Rust lol.


The Arduino serial monitor only showed 100 points at a time (ugh!) but I found https://github.com/hacknus/serial-monitor-rust which worked great.

uggh, serial monitor isn't actually live.
Seems like it must have some buffer or otherwise be dropping stuff on the floor.
Recording a few seconds and then saving a CSV only gives a 1000-ish data.

but using minicom

minicom -D /dev/tty.usbmodem4995277E384B1 -b 115200 -C foo.csv

gives 10x the data.



### 2024 July 29 - hardware connection test
Connected via stlink and jtag pins as per https://github.com/MitkoDyakov/BluePillCaliper/blob/main/Hardware/Schematics%20V1.1.pdf

curl --proto '=https' --tlsv1.2 -LsSf https://github.com/probe-rs/probe-rs/releases/latest/download/probe-rs-tools-installer.sh | sh


$ probe-rs info
Probing target via JTAG

WARN probe_rs::probe::stlink: send_jtag_command 242 failed: JtagGetIdcodeError
Error identifying target using protocol JTAG: An error with the usage of the probe occurred

Probing target via SWD

WARN probe_rs::probe::stlink: send_jtag_command 242 failed: JtagGetIdcodeError
Error identifying target using protocol SWD: An error with the usage of the probe occurred


ah, I was reading the schematic incorrectly. Managed to connect:

$ probe-rs info
Probing target via JTAG

ARM Chip with debug port Default:
Debug Port: DPv1, DP Designer: ARM Ltd
└── 0 MemoryAP
└── ROM Table (Class 1), Designer: STMicroelectronics
├── Cortex-M3 SCS   (Generic IP component)
│   └── CPUID
│       ├── IMPLEMENTER: ARM Ltd
│       ├── VARIANT: 1
│       ├── PARTNO: Cortex-M3
│       └── REVISION: 1
├── Cortex-M3 DWT   (Generic IP component)
├── Cortex-M3 FBP   (Generic IP component)
├── Cortex-M3 ITM   (Generic IP component)
└── Cortex-M3 TPIU  (Coresight Component)
